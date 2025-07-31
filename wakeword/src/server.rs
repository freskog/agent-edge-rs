use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use wakeword_protocol::{
    protocol::{AudioChunk, EndOfSpeechEvent, SubscriptionType, UtteranceSessionStarted},
    Connection, Message, ProtocolError, WakewordEvent,
};

/// Streaming message types that can be sent to subscribers
#[derive(Debug, Clone)]
pub enum StreamingMessage {
    WakewordEvent(WakewordEvent),
    AudioChunk(AudioChunk),
    EndOfSpeech(EndOfSpeechEvent),
    UtteranceSessionStarted(UtteranceSessionStarted),
}

/// Subscriber information with subscription type
#[derive(Debug)]
struct SubscriberInfo {
    sender: mpsc::Sender<StreamingMessage>,
    subscription_type: SubscriptionType,
    subscriber_id: String,
}

/// Handle for managing the wakeword event server
#[derive(Clone)]
pub struct WakewordServer {
    subscribers: Arc<Mutex<HashMap<String, SubscriberInfo>>>,
    next_subscriber_id: Arc<Mutex<u32>>,
}

impl WakewordServer {
    /// Create a new wakeword server
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            next_subscriber_id: Arc::new(Mutex::new(0)),
        }
    }

    /// Start the TCP server on the given address
    pub fn start(&self, bind_address: &str) -> Result<(), std::io::Error> {
        let listener = TcpListener::bind(bind_address)?;
        info!("🌐 Wakeword server listening on {}", bind_address);

        let server = self.clone();

        // Spawn server thread
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let server = server.clone();
                        thread::spawn(move || {
                            if let Err(e) = server.handle_client(stream) {
                                error!("❌ Error handling client: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("❌ Error accepting connection: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Handle a new client connection
    fn handle_client(&self, stream: TcpStream) -> Result<(), ProtocolError> {
        let peer_addr = stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        info!("🔌 New client connected: {}", peer_addr);

        // Set read timeout to handle blocking gracefully
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .map_err(ProtocolError::Io)?;

        let mut connection = Connection::new(stream)?;
        let mut event_receiver: Option<mpsc::Receiver<StreamingMessage>> = None;
        let mut subscriber_id: Option<String> = None;

        loop {
            // Check for incoming events to send to client
            if let Some(ref receiver) = event_receiver {
                match receiver.try_recv() {
                    Ok(streaming_message) => {
                        let message = match streaming_message {
                            StreamingMessage::WakewordEvent(event) => Message::WakewordEvent(event),
                            StreamingMessage::AudioChunk(chunk) => Message::AudioChunk(chunk),
                            StreamingMessage::EndOfSpeech(eos_event) => {
                                Message::EndOfSpeech(eos_event)
                            }
                            StreamingMessage::UtteranceSessionStarted(session) => {
                                Message::UtteranceSessionStarted(session)
                            }
                        };

                        if let Err(e) = connection.write_message(&message) {
                            error!("❌ Failed to send message to {}: {}", peer_addr, e);
                            break;
                        }
                        debug!(
                            "✅ Sent streaming message to {} ({})",
                            peer_addr,
                            subscriber_id.as_ref().unwrap_or(&"unknown".to_string())
                        );
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        // No events available, continue
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        info!(
                            "📡 Event channel closed, disconnecting client {}",
                            peer_addr
                        );
                        break;
                    }
                }
            }

            // Check for client messages
            match connection.read_message() {
                Ok(Message::SubscribeWakeword) => {
                    if subscriber_id.is_some() {
                        // Already subscribed
                        let response = Message::SubscribeResponse {
                            success: false,
                            message: "Already subscribed".to_string(),
                        };
                        connection.write_message(&response)?;
                        continue;
                    }

                    // Generate new subscriber ID
                    let id = {
                        let mut next_id = self.next_subscriber_id.lock().unwrap();
                        *next_id += 1;
                        format!("subscriber_{}", *next_id)
                    };

                    info!(
                        "📝 Client {} subscribed to wakeword events with ID: {}",
                        peer_addr, id
                    );

                    // Create channel for this subscriber
                    let (sender, receiver) = mpsc::channel();

                    // Add subscriber info to map
                    {
                        let mut subscribers = self.subscribers.lock().unwrap();
                        subscribers.insert(
                            id.clone(),
                            SubscriberInfo {
                                sender,
                                subscription_type: SubscriptionType::WakewordOnly,
                                subscriber_id: id.clone(),
                            },
                        );
                    }

                    event_receiver = Some(receiver);
                    subscriber_id = Some(id.clone());

                    let response = Message::SubscribeResponse {
                        success: true,
                        message: format!("Subscribed to wakeword events with ID: {}", id),
                    };
                    connection.write_message(&response)?;
                }
                Ok(Message::SubscribeUtterance(subscription_type)) => {
                    if subscriber_id.is_some() {
                        // Already subscribed
                        let response = Message::SubscribeResponse {
                            success: false,
                            message: "Already subscribed".to_string(),
                        };
                        connection.write_message(&response)?;
                        continue;
                    }

                    // Generate new subscriber ID
                    let id = {
                        let mut next_id = self.next_subscriber_id.lock().unwrap();
                        *next_id += 1;
                        format!("subscriber_{}", *next_id)
                    };

                    info!(
                        "📝 Client {} subscribed to utterance streaming ({:?}) with ID: {}",
                        peer_addr, subscription_type, id
                    );

                    // Create channel for this subscriber
                    let (sender, receiver) = mpsc::channel();

                    // Add subscriber info to map
                    {
                        let mut subscribers = self.subscribers.lock().unwrap();
                        subscribers.insert(
                            id.clone(),
                            SubscriberInfo {
                                sender,
                                subscription_type: subscription_type.clone(),
                                subscriber_id: id.clone(),
                            },
                        );
                    }

                    event_receiver = Some(receiver);
                    subscriber_id = Some(id.clone());

                    let response = Message::SubscribeResponse {
                        success: true,
                        message: format!(
                            "Subscribed to utterance streaming ({:?}) with ID: {}",
                            subscription_type, id
                        ),
                    };
                    connection.write_message(&response)?;
                }
                Ok(Message::UnsubscribeWakeword) | Ok(Message::UnsubscribeUtterance) => {
                    if let Some(id) = subscriber_id.take() {
                        // Remove from subscribers map
                        {
                            let mut subscribers = self.subscribers.lock().unwrap();
                            subscribers.remove(&id);
                        }

                        event_receiver = None;
                        info!("📝 Client {} unsubscribed (ID: {})", peer_addr, id);

                        let response = Message::UnsubscribeResponse {
                            success: true,
                            message: "Unsubscribed successfully".to_string(),
                        };
                        connection.write_message(&response)?;
                    } else {
                        let response = Message::UnsubscribeResponse {
                            success: false,
                            message: "Not subscribed".to_string(),
                        };
                        connection.write_message(&response)?;
                    }
                }
                Ok(msg) => {
                    warn!("⚠️ Unexpected message from {}: {:?}", peer_addr, msg);
                    let response = Message::ErrorResponse {
                        error: "Unexpected message type".to_string(),
                    };
                    connection.write_message(&response)?;
                }
                Err(e) => match e {
                    ProtocolError::Io(ref io_err) => match io_err.kind() {
                        std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted => {
                            info!("🔌 Client {} disconnected", peer_addr);
                            break;
                        }
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                            // Timeout is normal, continue checking for events
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        _ => {
                            error!("❌ IO error with client {}: {}", peer_addr, io_err);
                            break;
                        }
                    },
                    _ => {
                        error!("❌ Protocol error with client {}: {}", peer_addr, e);
                        break;
                    }
                },
            }
        }

        info!("🧹 Cleaned up client connection: {}", peer_addr);
        Ok(())
    }

    /// Broadcast a wakeword event to all subscribers (backward compatibility)
    pub fn broadcast_event(&self, event: WakewordEvent) {
        self.broadcast_message(StreamingMessage::WakewordEvent(event));
    }

    /// Broadcast a streaming message to appropriate subscribers
    pub fn broadcast_message(&self, message: StreamingMessage) {
        info!("📢 Starting broadcast_message");
        let subscribers = self.subscribers.lock().unwrap();
        info!(
            "📢 Acquired subscribers lock, {} total subscribers",
            subscribers.len()
        );
        let mut to_remove = Vec::new();

        if subscribers.is_empty() {
            debug!("📢 No active subscribers for streaming message");
            return;
        }

        // Filter subscribers based on message type and subscription
        let relevant_subscribers: Vec<_> = subscribers
            .iter()
            .filter(|(_, info)| self.should_send_message(&message, &info.subscription_type))
            .collect();

        if relevant_subscribers.is_empty() {
            debug!("📢 No relevant subscribers for message type");
            return;
        }

        info!(
            "📢 Broadcasting streaming message to {} relevant subscribers (out of {} total)",
            relevant_subscribers.len(),
            subscribers.len()
        );

        // Send message to relevant subscribers
        for (subscriber_id, info) in relevant_subscribers {
            info!("📢 Sending message to subscriber {}", subscriber_id);
            match info.sender.send(message.clone()) {
                Ok(_) => {
                    info!("✅ Sent message to subscriber {}", subscriber_id);
                }
                Err(_) => {
                    info!(
                        "❌ Failed to send message to subscriber {} (disconnected)",
                        subscriber_id
                    );
                    to_remove.push(subscriber_id.clone());
                }
            }
        }

        info!("📢 Finished sending to all subscribers");

        // Remove disconnected subscribers
        if !to_remove.is_empty() {
            drop(subscribers); // Release the lock before acquiring it again
            let mut subscribers = self.subscribers.lock().unwrap();
            for subscriber_id in &to_remove {
                subscribers.remove(subscriber_id);
            }
            info!("🧹 Removed {} disconnected subscribers", to_remove.len());
        }

        info!("📢 Broadcast complete");
    }

    /// Check if a message should be sent to a subscriber based on their subscription type
    fn should_send_message(
        &self,
        message: &StreamingMessage,
        subscription_type: &SubscriptionType,
    ) -> bool {
        match (message, subscription_type) {
            // WakewordOnly subscribers only get wake word events
            (StreamingMessage::WakewordEvent(_), SubscriptionType::WakewordOnly) => true,

            // WakewordPlusUtterance subscribers get all messages
            (_, SubscriptionType::WakewordPlusUtterance) => true,

            // UtteranceOnly subscribers get audio and EOS messages (no wake word events)
            (StreamingMessage::AudioChunk(_), SubscriptionType::UtteranceOnly) => true,
            (StreamingMessage::EndOfSpeech(_), SubscriptionType::UtteranceOnly) => true,
            (StreamingMessage::UtteranceSessionStarted(_), SubscriptionType::UtteranceOnly) => true,

            // All other combinations are filtered out
            _ => false,
        }
    }

    /// Check if there are any subscribers who want utterance streaming
    pub fn has_utterance_subscribers(&self) -> bool {
        let subscribers = self.subscribers.lock().unwrap();
        let utterance_count = subscribers
            .values()
            .filter(|info| {
                matches!(
                    info.subscription_type,
                    SubscriptionType::WakewordPlusUtterance | SubscriptionType::UtteranceOnly
                )
            })
            .count();

        info!(
            "🔍 Server has {} total subscribers, {} want utterance streaming",
            subscribers.len(),
            utterance_count
        );

        for (id, info) in subscribers.iter() {
            info!("🔍 Subscriber {}: type={:?}", id, info.subscription_type);
        }

        utterance_count > 0
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }

    /// Get the number of subscribers by type
    pub fn subscriber_count_by_type(&self, subscription_type: &SubscriptionType) -> usize {
        let subscribers = self.subscribers.lock().unwrap();
        subscribers
            .values()
            .filter(|info| &info.subscription_type == subscription_type)
            .count()
    }
}
