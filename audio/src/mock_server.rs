use audio_protocol::{AudioChunk, Connection, Message, ProtocolError};
use hound::WavReader;
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const CHUNK_SIZE: usize = 1280; // 80ms at 16kHz (1280 samples = 2560 bytes for i16)

/// Configuration for the mock audio server
#[derive(Debug, Clone)]
pub struct MockServerConfig {
    /// Wave file to play (should be 16kHz mono s16le)
    pub audio_file: PathBuf,
    /// Address to bind the TCP server to (use "127.0.0.1:0" for random port)
    pub bind_address: String,
    /// Loop the audio file continuously
    pub loop_audio: bool,
    /// Silence duration after file ends before looping (in seconds)
    pub silence_duration: f32,
    /// Playback speed multiplier (1.0 = real time, 2.0 = 2x speed)
    pub speed: f32,
}

impl Default for MockServerConfig {
    fn default() -> Self {
        Self {
            audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
            bind_address: "127.0.0.1:0".to_string(), // Random port
            loop_audio: false,
            silence_duration: 2.0,
            speed: 1.0,
        }
    }
}

/// Client connection info
#[derive(Debug)]
struct ClientInfo {
    stream: TcpStream,
    subscribed: bool,
}

/// Mock audio server that serves a single wave file to all clients
pub struct MockAudioServer {
    config: MockServerConfig,
    should_stop: Arc<AtomicBool>,
    clients: Arc<Mutex<HashMap<usize, Arc<Mutex<ClientInfo>>>>>,
    next_client_id: Arc<AtomicUsize>,
    actual_port: Option<u16>,
}

impl MockAudioServer {
    pub fn new(config: MockServerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Verify the audio file exists and has correct format
        Self::verify_audio_file(&config.audio_file)?;

        info!("🎵 Mock audio server configured:");
        info!("  📁 File: {}", config.audio_file.display());
        info!("  🔄 Loop: {}", config.loop_audio);
        info!("  ⏱️ Speed: {}x", config.speed);
        info!("  🔇 Silence duration: {:.1}s", config.silence_duration);

        Ok(Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            clients: Arc::new(Mutex::new(HashMap::new())),
            next_client_id: Arc::new(AtomicUsize::new(1)),
            actual_port: None,
        })
    }

    fn verify_audio_file(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = WavReader::new(BufReader::new(file))?;
        let spec = reader.spec();

        info!(
            "📊 Audio file info: {}Hz, {} channels, {} bits",
            spec.sample_rate, spec.channels, spec.bits_per_sample
        );

        if spec.sample_rate != 16000 {
            return Err(format!("Expected 16kHz sample rate, got {}Hz", spec.sample_rate).into());
        }
        if spec.channels != 1 {
            return Err(format!("Expected mono audio, got {} channels", spec.channels).into());
        }
        if spec.bits_per_sample != 16 {
            return Err(format!("Expected 16-bit audio, got {} bits", spec.bits_per_sample).into());
        }

        Ok(())
    }

    /// Start the server and return the actual bound port
    pub fn start(&mut self) -> Result<u16, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.config.bind_address)?;
        let actual_port = listener.local_addr()?.port();
        self.actual_port = Some(actual_port);

        info!(
            "🎵 Mock audio server listening on 127.0.0.1:{}",
            actual_port
        );

        // Start audio streaming thread
        let _audio_thread = self.start_audio_thread();

        // Accept client connections in background thread
        let should_stop = self.should_stop.clone();
        let clients = self.clients.clone();
        let next_client_id = self.next_client_id.clone();

        thread::spawn(move || {
            for stream in listener.incoming() {
                if should_stop.load(Ordering::Relaxed) {
                    break;
                }

                match stream {
                    Ok(stream) => {
                        let client_id = next_client_id.fetch_add(1, Ordering::Relaxed);
                        info!("📡 Client {} connected", client_id);

                        let client_info = ClientInfo {
                            stream: stream.try_clone().unwrap(),
                            subscribed: false,
                        };

                        {
                            let mut clients_guard = clients.lock().unwrap();
                            clients_guard.insert(client_id, Arc::new(Mutex::new(client_info)));
                        }

                        let clients = clients.clone();
                        let should_stop = should_stop.clone();

                        thread::spawn(move || {
                            if let Err(e) =
                                Self::handle_client(stream, client_id, clients, should_stop)
                            {
                                error!("Client {} error: {}", client_id, e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Connection error: {}", e);
                    }
                }
            }
        });

        Ok(actual_port)
    }

    /// Start the server in a background thread and return a handle for testing
    pub fn start_background(mut self) -> Result<MockServerHandle, Box<dyn std::error::Error>> {
        let port = self.start()?;
        let should_stop = self.should_stop.clone();

        Ok(MockServerHandle {
            port,
            should_stop,
            _server: self,
        })
    }

    fn start_audio_thread(&self) -> thread::JoinHandle<()> {
        let audio_file = self.config.audio_file.clone();
        let loop_audio = self.config.loop_audio;
        let silence_duration = Duration::from_secs_f32(self.config.silence_duration);
        let chunk_interval = Duration::from_millis((80.0 / self.config.speed) as u64);
        let should_stop = self.should_stop.clone();
        let clients = self.clients.clone();

        thread::spawn(move || {
            info!("🎶 Starting audio streaming thread");

            loop {
                if should_stop.load(Ordering::Relaxed) {
                    break;
                }

                // Check if we have any subscribed clients
                let subscriber_count = {
                    let clients = clients.lock().unwrap();
                    clients
                        .values()
                        .filter(|client| client.lock().unwrap().subscribed)
                        .count()
                };

                if subscriber_count == 0 {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }

                info!("▶️ Playing audio file: {}", audio_file.display());

                // Play the audio file
                if let Err(e) =
                    Self::stream_audio_file(&audio_file, &clients, chunk_interval, &should_stop)
                {
                    error!("Error streaming audio: {}", e);
                    break;
                }

                if !loop_audio {
                    info!("🏁 Finished playing file (no loop)");
                    break;
                }

                // Silence period between loops
                info!("🔇 Silence period ({:?})", silence_duration);
                let silence_start = Instant::now();
                while silence_start.elapsed() < silence_duration {
                    if should_stop.load(Ordering::Relaxed) {
                        return;
                    }

                    // Send silence chunk
                    let silence_chunk = vec![0u8; CHUNK_SIZE * 2]; // 2 bytes per i16 sample
                    Self::broadcast_audio_chunk(&silence_chunk, &clients);
                    thread::sleep(chunk_interval);
                }
            }

            info!("🛑 Audio streaming thread stopped");
        })
    }

    fn stream_audio_file(
        file_path: &PathBuf,
        clients: &Arc<Mutex<HashMap<usize, Arc<Mutex<ClientInfo>>>>>,
        chunk_interval: Duration,
        should_stop: &Arc<AtomicBool>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::open(file_path)?;
        let mut reader = WavReader::new(BufReader::new(file))?;

        let mut buffer = Vec::with_capacity(CHUNK_SIZE * 2); // Buffer for raw bytes
        let mut chunk_count = 0;

        loop {
            if should_stop.load(Ordering::Relaxed) {
                break;
            }

            buffer.clear();

            // Read CHUNK_SIZE samples (each sample is 2 bytes for i16)
            for _ in 0..CHUNK_SIZE {
                match reader.samples::<i16>().next() {
                    Some(Ok(sample)) => {
                        buffer.extend_from_slice(&sample.to_le_bytes());
                    }
                    Some(Err(e)) => {
                        return Err(format!("Error reading sample: {}", e).into());
                    }
                    None => {
                        // End of file - pad with silence if needed
                        while buffer.len() < CHUNK_SIZE * 2 {
                            buffer.extend_from_slice(&0i16.to_le_bytes());
                        }
                        debug!("📄 Reached end of audio file after {} chunks", chunk_count);

                        // Send final chunk and return
                        Self::broadcast_audio_chunk(&buffer, clients);
                        return Ok(());
                    }
                }
            }

            // Broadcast this chunk to all subscribed clients
            Self::broadcast_audio_chunk(&buffer, clients);
            chunk_count += 1;

            if chunk_count % 100 == 0 {
                debug!("🎵 Sent {} audio chunks", chunk_count);
            }

            thread::sleep(chunk_interval);
        }

        Ok(())
    }

    fn broadcast_audio_chunk(
        data: &[u8],
        clients: &Arc<Mutex<HashMap<usize, Arc<Mutex<ClientInfo>>>>>,
    ) {
        let audio_chunk = AudioChunk {
            data: data.to_vec(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        let message = Message::AudioChunk {
            audio_data: audio_chunk.data,
            timestamp_ms: audio_chunk.timestamp_ms,
        };

        // Send to all subscribed clients
        let clients_guard = clients.lock().unwrap();
        let mut disconnected_clients = Vec::new();

        for (client_id, client_info) in clients_guard.iter() {
            let client = client_info.lock().unwrap();
            if client.subscribed {
                let mut connection = Connection::new(client.stream.try_clone().unwrap()).unwrap();
                if let Err(e) = connection.write_message(&message) {
                    debug!("Client {} disconnected: {}", client_id, e);
                    disconnected_clients.push(*client_id);
                }
            }
        }

        // Clean up disconnected clients
        drop(clients_guard);
        if !disconnected_clients.is_empty() {
            let mut clients_guard = clients.lock().unwrap();
            for client_id in disconnected_clients {
                clients_guard.remove(&client_id);
                info!("🔌 Removed disconnected client {}", client_id);
            }
        }
    }

    fn handle_client(
        stream: TcpStream,
        client_id: usize,
        clients: Arc<Mutex<HashMap<usize, Arc<Mutex<ClientInfo>>>>>,
        should_stop: Arc<AtomicBool>,
    ) -> Result<(), ProtocolError> {
        let mut connection = Connection::new(stream)?;

        loop {
            if should_stop.load(Ordering::Relaxed) {
                break;
            }

            match connection.read_message() {
                Ok(Message::SubscribeAudio) => {
                    // Mark client as subscribed
                    {
                        let clients = clients.lock().unwrap();
                        if let Some(client_info) = clients.get(&client_id) {
                            client_info.lock().unwrap().subscribed = true;
                        }
                    }
                    info!("📡 Client {} subscribed to audio", client_id);
                }
                Ok(Message::UnsubscribeAudio) => {
                    // Mark client as unsubscribed
                    {
                        let clients = clients.lock().unwrap();
                        if let Some(client_info) = clients.get(&client_id) {
                            client_info.lock().unwrap().subscribed = false;
                        }
                    }

                    let response = Message::UnsubscribeResponse {
                        success: true,
                        message: "Unsubscribed from mock audio".to_string(),
                    };
                    connection.write_message(&response)?;
                    info!("📡 Client {} unsubscribed from audio", client_id);
                }
                Ok(msg) => {
                    warn!("⚠️ Client {} sent unexpected message: {:?}", client_id, msg);
                }
                Err(e) => {
                    debug!("Client {} disconnected: {}", client_id, e);
                    break;
                }
            }
        }

        // Remove client on disconnect
        {
            let mut clients = clients.lock().unwrap();
            clients.remove(&client_id);
        }
        info!("🔌 Client {} disconnected", client_id);
        Ok(())
    }

    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::Relaxed);
    }

    pub fn port(&self) -> Option<u16> {
        self.actual_port
    }
}

/// Handle for a mock server running in the background
pub struct MockServerHandle {
    pub port: u16,
    should_stop: Arc<AtomicBool>,
    _server: MockAudioServer,
}

impl MockServerHandle {
    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        self.stop();
        // Give it a moment to shut down gracefully
        thread::sleep(Duration::from_millis(100));
    }
}
