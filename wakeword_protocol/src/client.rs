use crate::protocol::{Connection, Message, ProtocolError, WakewordEvent};
use log::{debug, error, info, warn};
use std::net::TcpStream;
use std::time::Duration;

/// Result type for subscription operations
#[derive(Debug)]
pub enum SubscribeResult {
    Success,
    AlreadySubscribed,
    Error(String),
}

/// High-level TCP client for wakeword event subscription
pub struct WakewordClient {
    connection: Connection,
    server_address: String,
    is_subscribed: bool,
}

impl WakewordClient {
    /// Connect to the wakeword server
    pub fn connect(address: &str) -> Result<Self, ProtocolError> {
        info!("📡 Connecting to wakeword server at {}", address);

        let stream = TcpStream::connect(address)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?; // Long timeout for low CPU usage
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        let connection = Connection::new(stream)?;

        info!("✅ Connected to wakeword server");

        Ok(WakewordClient {
            connection,
            server_address: address.to_string(),
            is_subscribed: false,
        })
    }

    /// Subscribe to wakeword events
    pub fn subscribe_wakeword(&mut self) -> Result<SubscribeResult, ProtocolError> {
        if self.is_subscribed {
            return Ok(SubscribeResult::AlreadySubscribed);
        }

        debug!("📤 Sending SubscribeWakeword message");

        let message = Message::SubscribeWakeword;
        self.connection.write_message(&message)?;

        // Read response
        match self.connection.read_message()? {
            Message::SubscribeResponse { success, message } => {
                if success {
                    self.is_subscribed = true;
                    info!("🔔 Successfully subscribed to wakeword events");
                    Ok(SubscribeResult::Success)
                } else {
                    warn!("⚠️ Failed to subscribe to wakeword events: {}", message);
                    Ok(SubscribeResult::Error(message))
                }
            }
            Message::ErrorResponse { error } => {
                error!("❌ Server error during subscription: {}", error);
                Ok(SubscribeResult::Error(error))
            }
            msg => {
                let error = format!("Unexpected response to subscription: {:?}", msg);
                error!("❌ {}", error);
                Ok(SubscribeResult::Error(error))
            }
        }
    }

    /// Unsubscribe from wakeword events
    pub fn unsubscribe_wakeword(&mut self) -> Result<SubscribeResult, ProtocolError> {
        if !self.is_subscribed {
            return Ok(SubscribeResult::Success); // Already unsubscribed
        }

        debug!("📤 Sending UnsubscribeWakeword message");

        let message = Message::UnsubscribeWakeword;
        self.connection.write_message(&message)?;

        // Read response
        match self.connection.read_message()? {
            Message::UnsubscribeResponse { success, message } => {
                if success {
                    self.is_subscribed = false;
                    info!("🔕 Successfully unsubscribed from wakeword events");
                    Ok(SubscribeResult::Success)
                } else {
                    warn!("⚠️ Failed to unsubscribe from wakeword events: {}", message);
                    Ok(SubscribeResult::Error(message))
                }
            }
            Message::ErrorResponse { error } => {
                error!("❌ Server error during unsubscription: {}", error);
                Ok(SubscribeResult::Error(error))
            }
            msg => {
                let error = format!("Unexpected response to unsubscription: {:?}", msg);
                error!("❌ {}", error);
                Ok(SubscribeResult::Error(error))
            }
        }
    }

    /// Read a wakeword event (blocking)
    /// Returns `None` if the connection is closed or an error occurs
    pub fn read_wakeword_event(&mut self) -> Result<Option<WakewordEvent>, ProtocolError> {
        if !self.is_subscribed {
            warn!("⚠️ Attempting to read wakeword events without subscription");
            return Ok(None);
        }

        match self.connection.read_message() {
            Ok(Message::WakewordEvent(event)) => {
                debug!(
                    "🎯 Received wakeword event: '{}' confidence {:.3}",
                    event.model_name, event.confidence
                );
                Ok(Some(event))
            }
            Ok(Message::ErrorResponse { error }) => {
                error!("❌ Server error: {}", error);
                Ok(None)
            }
            Ok(msg) => {
                warn!("⚠️ Unexpected message while reading events: {:?}", msg);
                Ok(None)
            }
            Err(e) => {
                match e {
                    ProtocolError::Io(ref io_err) => {
                        match io_err.kind() {
                            std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted => {
                                info!("🔌 Connection closed by server");
                                self.is_subscribed = false;
                                Ok(None)
                            }
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                                // Timeout is normal for responsiveness, no need to log frequently
                                Ok(None) // Timeout is normal, keep trying
                            }
                            _ => {
                                error!("❌ IO error reading wakeword event: {}", io_err);
                                Err(e)
                            }
                        }
                    }
                    _ => {
                        error!("❌ Protocol error reading wakeword event: {}", e);
                        Err(e)
                    }
                }
            }
        }
    }

    /// Check if currently subscribed to wakeword events
    pub fn is_subscribed(&self) -> bool {
        self.is_subscribed
    }

    /// Get the server address this client is connected to
    pub fn server_address(&self) -> &str {
        &self.server_address
    }

    /// Attempt to reconnect to the server
    pub fn reconnect(&mut self) -> Result<(), ProtocolError> {
        info!(
            "🔄 Reconnecting to wakeword server at {}",
            self.server_address
        );

        let stream = TcpStream::connect(&self.server_address)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?; // Long timeout for low CPU usage
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        self.connection = Connection::new(stream)?;
        self.is_subscribed = false; // Need to resubscribe after reconnection

        info!("✅ Reconnected to wakeword server");
        Ok(())
    }

    /// Listen for wakeword events with a callback function
    /// This is a convenience method that handles the event loop
    pub fn listen_for_events<F>(&mut self, mut callback: F) -> Result<(), ProtocolError>
    where
        F: FnMut(WakewordEvent),
    {
        if !self.is_subscribed {
            self.subscribe_wakeword()?;
        }

        info!("👂 Starting to listen for wakeword events...");

        loop {
            match self.read_wakeword_event()? {
                Some(event) => {
                    callback(event);
                }
                None => {
                    // Connection lost or timeout, try to continue
                    if !self.is_subscribed {
                        warn!("📡 Lost subscription, attempting to reconnect...");
                        match self.reconnect() {
                            Ok(()) => match self.subscribe_wakeword()? {
                                SubscribeResult::Success => {
                                    info!("🔔 Resubscribed successfully");
                                }
                                result => {
                                    error!("❌ Failed to resubscribe: {:?}", result);
                                    return Err(ProtocolError::Io(std::io::Error::new(
                                        std::io::ErrorKind::ConnectionRefused,
                                        "Failed to resubscribe after reconnection",
                                    )));
                                }
                            },
                            Err(e) => {
                                error!("❌ Failed to reconnect: {}", e);
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }
    }
}
