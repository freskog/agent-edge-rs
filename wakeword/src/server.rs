use log::{debug, error, info, warn};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use wakeword_protocol::{Connection, Message, ProtocolError, WakewordEvent};

/// Handle for managing the wakeword event server
#[derive(Clone)]
pub struct WakewordServer {
    subscribers: Arc<Mutex<Vec<mpsc::Sender<WakewordEvent>>>>,
    next_subscriber_id: Arc<Mutex<u32>>,
}

impl WakewordServer {
    /// Create a new wakeword server
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
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
        let mut event_receiver: Option<mpsc::Receiver<WakewordEvent>> = None;
        let mut subscriber_id: Option<String> = None;

        loop {
            // Check for incoming events to send to client
            if let Some(ref receiver) = event_receiver {
                match receiver.try_recv() {
                    Ok(event) => {
                        let message = Message::WakewordEvent(event.clone());
                        if let Err(e) = connection.write_message(&message) {
                            error!("❌ Failed to send event to {}: {}", peer_addr, e);
                            break;
                        }
                        debug!(
                            "✅ Sent event to {} ({})",
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

                    info!("📝 Client {} subscribed with ID: {}", peer_addr, id);

                    // Create channel for this subscriber
                    let (sender, receiver) = mpsc::channel();

                    // Add sender to subscribers list
                    {
                        let mut subscribers = self.subscribers.lock().unwrap();
                        subscribers.push(sender);
                    }

                    event_receiver = Some(receiver);
                    subscriber_id = Some(id.clone());

                    let response = Message::SubscribeResponse {
                        success: true,
                        message: format!("Subscribed with ID: {}", id),
                    };
                    connection.write_message(&response)?;
                }
                Ok(Message::UnsubscribeWakeword) => {
                    if subscriber_id.take().is_some() {
                        event_receiver = None;
                        info!("📝 Client {} unsubscribed", peer_addr);

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

    /// Broadcast a wakeword event to all subscribers
    pub fn broadcast_event(&self, event: WakewordEvent) {
        let mut subscribers = self.subscribers.lock().unwrap();
        let mut to_remove = Vec::new();

        if subscribers.is_empty() {
            debug!(
                "📢 No active subscribers for wakeword event: {}",
                event.model_name
            );
            return;
        }

        debug!(
            "📢 Broadcasting wakeword event to {} subscribers",
            subscribers.len()
        );

        // Send event to all subscribers
        for (index, sender) in subscribers.iter().enumerate() {
            match sender.send(event.clone()) {
                Ok(_) => {
                    debug!("✅ Sent event to subscriber {}", index);
                }
                Err(_) => {
                    debug!(
                        "❌ Failed to send event to subscriber {} (disconnected)",
                        index
                    );
                    to_remove.push(index);
                }
            }
        }

        // Remove disconnected subscribers (in reverse order to maintain indices)
        for &index in to_remove.iter().rev() {
            subscribers.remove(index);
        }

        if !to_remove.is_empty() {
            info!("🧹 Removed {} disconnected subscribers", to_remove.len());
        }

        let active_count = subscribers.len();
        if active_count > 0 {
            info!(
                "📢 Broadcasted wakeword event '{}' to {} subscribers",
                event.model_name, active_count
            );
        }
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }
}
