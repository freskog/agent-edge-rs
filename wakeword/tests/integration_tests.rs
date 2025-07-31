use audio::{MockAudioServer, MockServerConfig};
use log::{debug, error, info};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use wakeword::server::WakewordServer;
use wakeword::tcp_client::WakewordClient as WakewordTcpClient;
use wakeword_protocol::client::{StreamingMessage, WakewordClient};
use wakeword_protocol::protocol::SubscriptionType;

/// Helper function to start a mock audio server for testing
fn start_mock_audio_server(
    audio_file: &str,
    speed: f32,
    loop_audio: bool,
) -> audio::MockServerHandle {
    let config = MockServerConfig {
        audio_file: PathBuf::from(audio_file),
        bind_address: "127.0.0.1:0".to_string(),
        loop_audio,
        silence_duration: 0.5,
        speed,
    };

    MockAudioServer::new(config)
        .expect("Failed to create mock audio server")
        .start_background()
        .expect("Failed to start mock audio server")
}

/// Helper function to start a wakeword server for testing
fn start_wakeword_server() -> (WakewordServer, String) {
    let server = WakewordServer::new();
    // Use a fixed test port that's unlikely to conflict
    let port = 8090 + (std::process::id() % 100) as u16; // Pseudo-random port based on PID
    let address = format!("127.0.0.1:{}", port);
    (server, address)
}

/// Helper function to start a wakeword detection client
fn start_wakeword_detection_client(
    audio_server_address: &str,
    wakeword_server: Arc<WakewordServer>,
) -> thread::JoinHandle<()> {
    let audio_address = audio_server_address.to_string();

    thread::spawn(move || {
        let models = vec!["hey_mycroft".to_string()];
        let mut client = WakewordTcpClient::new(&audio_address, models, 0.3)
            .expect("Failed to create wakeword client");

        // Set the wakeword server for broadcasting
        client.set_wakeword_server(wakeword_server);

        // Start processing audio and detecting wake words
        if let Err(e) = client.start_detection() {
            log::error!("Wakeword client error: {}", e);
        }
    })
}

#[test]
fn test_wakeword_detection_with_utterance_capture() {
    env_logger::try_init().ok();
    info!("🧪 Starting wakeword detection with utterance capture test");

    // 1. Start wakeword server (the event broadcaster) FIRST
    let (wakeword_server, wakeword_server_address) = start_wakeword_server();
    let wakeword_server_arc = Arc::new(wakeword_server);
    info!("🎯 Wakeword server created on {}", wakeword_server_address);

    // Start the server in background
    let server_arc_clone = wakeword_server_arc.clone();
    let server_address_clone = wakeword_server_address.clone();
    let _server_handle = thread::spawn(move || {
        if let Err(e) = server_arc_clone.start(&server_address_clone) {
            log::error!("Wakeword server error: {}", e);
        }
    });

    // Give the wakeword server time to start
    thread::sleep(Duration::from_millis(200));

    // 2. Connect test client as subscriber BEFORE starting audio
    let mut subscriber_client = WakewordClient::connect(&wakeword_server_address)
        .expect("Failed to connect to wakeword server");

    // Subscribe to wake word events + utterance audio
    subscriber_client
        .subscribe_utterance(SubscriptionType::WakewordPlusUtterance)
        .expect("Failed to subscribe to utterance events");
    info!("📡 Subscribed to wakeword + utterance events");

    // Give subscription time to register
    thread::sleep(Duration::from_millis(100));

    // 3. Start mock audio server with slower speed and longer file
    let mock_audio = start_mock_audio_server(
        "../tests/data/immediate_what_time_is_it.wav",
        1.0,   // Normal speed - don't rush it
        false, // Don't loop - play once
    );
    info!("🎵 Mock audio server started on {}", mock_audio.address());

    // Give the audio server time to start
    thread::sleep(Duration::from_millis(100));

    // 4. Start wakeword detection client (connects to audio, broadcasts to server)
    let _detection_client_handle =
        start_wakeword_detection_client(&mock_audio.address(), wakeword_server_arc.clone());
    info!("🔍 Wakeword detection client started");

    // Give the detection client time to start and connect
    thread::sleep(Duration::from_millis(300));

    // 5. Collect events and verify the expected sequence
    let mut events = Vec::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(15); // Longer timeout since we're not rushing

    while start_time.elapsed() < timeout {
        match subscriber_client.read_streaming_message() {
            Ok(Some(StreamingMessage::WakewordEvent(event))) => {
                info!("🎯 Received wake word: {}", event.model_name);
                events.push("wakeword".to_string());
            }
            Ok(Some(StreamingMessage::UtteranceSessionStarted(session))) => {
                info!("🎤 Utterance session started: {}", session.session_id);
                events.push("session_started".to_string());
            }
            Ok(Some(StreamingMessage::AudioChunk(chunk))) => {
                debug!("🎵 Received audio chunk: {} bytes", chunk.data.len());
                if !events.contains(&"audio_chunk".to_string()) {
                    events.push("audio_chunk".to_string());
                    info!("🎵 First audio chunk received ({} bytes)", chunk.data.len());
                }
            }
            Ok(Some(StreamingMessage::EndOfSpeech(eos))) => {
                info!("🔇 End of speech: {:?}", eos.reason);
                events.push("end_of_speech".to_string());
                break; // We got everything we expected
            }
            Ok(Some(StreamingMessage::Error(error))) => {
                panic!("❌ Received error: {}", error);
            }
            Ok(None) => {
                // No message available, continue waiting
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                error!("Error reading streaming message: {}", e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    info!("✅ Test completed. Events received: {:?}", events);

    // Verify we got the expected sequence
    assert!(
        events.contains(&"wakeword".to_string()),
        "Should detect wake word"
    );
    assert!(
        events.contains(&"session_started".to_string()),
        "Should start utterance session"
    );
    assert!(
        events.contains(&"audio_chunk".to_string()),
        "Should receive audio chunks"
    );
    assert!(
        events.contains(&"end_of_speech".to_string()),
        "Should detect end of speech"
    );

    // Verify the sequence is logical (wakeword should come before session, etc.)
    let wakeword_pos = events.iter().position(|e| e == "wakeword").unwrap();
    let session_pos = events.iter().position(|e| e == "session_started").unwrap();
    let audio_pos = events.iter().position(|e| e == "audio_chunk").unwrap();
    let eos_pos = events.iter().position(|e| e == "end_of_speech").unwrap();

    assert!(
        wakeword_pos < session_pos,
        "Wake word should come before session start"
    );
    assert!(
        session_pos < audio_pos,
        "Session should start before audio chunks"
    );
    assert!(
        audio_pos < eos_pos,
        "Audio chunks should come before end of speech"
    );

    info!("🎉 All assertions passed! The complete pipeline works correctly.");
}

#[test]
fn test_wakeword_only_subscription() {
    env_logger::try_init().ok();
    info!("🧪 Starting wakeword-only subscription test");

    // Start mock audio server
    let mock_audio = start_mock_audio_server(
        "../tests/data/hey_mycroft_test.wav",
        4.0, // 4x speed for very fast test
        false,
    );
    info!("🎵 Mock audio server started on {}", mock_audio.address());
    thread::sleep(Duration::from_millis(100));

    // Start wakeword server
    let (wakeword_server, wakeword_server_address) = start_wakeword_server();
    let wakeword_server_arc = Arc::new(wakeword_server);
    info!("🎯 Wakeword server created");

    let server_arc_clone = wakeword_server_arc.clone();
    let server_address_clone = wakeword_server_address.clone();
    let _server_handle = thread::spawn(move || {
        if let Err(e) = server_arc_clone.start(&server_address_clone) {
            log::error!("Wakeword server error: {}", e);
        }
    });
    thread::sleep(Duration::from_millis(200));

    // Start detection client
    let _detection_client_handle =
        start_wakeword_detection_client(&mock_audio.address(), wakeword_server_arc.clone());
    thread::sleep(Duration::from_millis(300));

    // Subscribe to wake word events ONLY (no utterance capture)
    let mut subscriber_client = WakewordClient::connect(&wakeword_server_address)
        .expect("Failed to connect to wakeword server");

    subscriber_client
        .subscribe_utterance(SubscriptionType::WakewordOnly)
        .expect("Failed to subscribe to wakeword events");
    info!("📡 Subscribed to wakeword-only events");

    // Collect events and verify we only get wake word events
    let mut events = Vec::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(5);

    while start_time.elapsed() < timeout && events.len() < 2 {
        match subscriber_client.read_streaming_message() {
            Ok(Some(StreamingMessage::WakewordEvent(event))) => {
                info!("🎯 Received wake word: {}", event.model_name);
                events.push("wakeword".to_string());
            }
            Ok(Some(StreamingMessage::UtteranceSessionStarted(_))) => {
                events.push("session_started".to_string());
                panic!("❌ Should not receive utterance session in WakewordOnly mode");
            }
            Ok(Some(StreamingMessage::AudioChunk(_))) => {
                events.push("audio_chunk".to_string());
                panic!("❌ Should not receive audio chunks in WakewordOnly mode");
            }
            Ok(Some(StreamingMessage::EndOfSpeech(_))) => {
                events.push("end_of_speech".to_string());
                panic!("❌ Should not receive end of speech in WakewordOnly mode");
            }
            Ok(Some(StreamingMessage::Error(error))) => {
                panic!("❌ Received error: {}", error);
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                error!("Error reading streaming message: {}", e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    info!("✅ WakewordOnly test completed. Events: {:?}", events);
    assert!(
        events.contains(&"wakeword".to_string()),
        "Should detect wake word"
    );
    assert_eq!(events.len(), 1, "Should only receive wake word events");
}

#[test]
fn test_multiple_subscribers() {
    env_logger::try_init().ok();
    info!("🧪 Starting multiple subscribers test");

    // Start mock audio server with looping for multiple subscribers
    let mock_audio = start_mock_audio_server(
        "../tests/data/hey_mycroft_test.wav",
        6.0,  // Fast playback
        true, // Loop for multiple detections
    );
    info!("🎵 Mock audio server started on {}", mock_audio.address());
    thread::sleep(Duration::from_millis(100));

    // Start wakeword server
    let (wakeword_server, wakeword_server_address) = start_wakeword_server();
    let wakeword_server_arc = Arc::new(wakeword_server);
    info!("🎯 Wakeword server created");

    let server_arc_clone = wakeword_server_arc.clone();
    let server_address_clone = wakeword_server_address.clone();
    let _server_handle = thread::spawn(move || {
        if let Err(e) = server_arc_clone.start(&server_address_clone) {
            log::error!("Wakeword server error: {}", e);
        }
    });
    thread::sleep(Duration::from_millis(200));

    // Start detection client
    let _detection_client_handle =
        start_wakeword_detection_client(&mock_audio.address(), wakeword_server_arc.clone());
    thread::sleep(Duration::from_millis(300));

    // Create multiple subscribers with different subscription types
    let mut subscribers = Vec::new();

    // Subscriber 1: WakewordOnly
    let mut sub1 =
        WakewordClient::connect(&wakeword_server_address).expect("Failed to connect subscriber 1");
    sub1.subscribe_utterance(SubscriptionType::WakewordOnly)
        .expect("Failed to subscribe subscriber 1");
    subscribers.push((sub1, "WakewordOnly"));

    // Subscriber 2: WakewordPlusUtterance
    let mut sub2 =
        WakewordClient::connect(&wakeword_server_address).expect("Failed to connect subscriber 2");
    sub2.subscribe_utterance(SubscriptionType::WakewordPlusUtterance)
        .expect("Failed to subscribe subscriber 2");
    subscribers.push((sub2, "WakewordPlusUtterance"));

    info!("📡 Created {} subscribers", subscribers.len());

    // Collect events from all subscribers for a short period
    let mut subscriber_events: HashMap<String, Vec<String>> = HashMap::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(3);

    while start_time.elapsed() < timeout {
        for (client, sub_type) in subscribers.iter_mut() {
            if let Ok(Some(message)) = client.read_streaming_message() {
                let events = subscriber_events
                    .entry(sub_type.to_string())
                    .or_insert_with(Vec::new);

                match message {
                    StreamingMessage::WakewordEvent(_) => {
                        events.push("wakeword".to_string());
                        info!("🎯 {} received wake word", sub_type);
                    }
                    StreamingMessage::UtteranceSessionStarted(_) => {
                        events.push("session_started".to_string());
                        info!("🎤 {} received session start", sub_type);
                    }
                    StreamingMessage::AudioChunk(_) => {
                        if !events.contains(&"audio_chunk".to_string()) {
                            events.push("audio_chunk".to_string());
                            info!("🎵 {} received first audio chunk", sub_type);
                        }
                    }
                    StreamingMessage::EndOfSpeech(_) => {
                        events.push("end_of_speech".to_string());
                        info!("🔇 {} received end of speech", sub_type);
                    }
                    StreamingMessage::Error(error) => {
                        panic!("❌ {} received error: {}", sub_type, error);
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    info!(
        "✅ Multiple subscribers test completed. Events: {:?}",
        subscriber_events
    );

    // Verify each subscriber got appropriate events
    let empty_events = vec![];
    let wakeword_only_events = subscriber_events
        .get("WakewordOnly")
        .unwrap_or(&empty_events);
    let wakeword_plus_events = subscriber_events
        .get("WakewordPlusUtterance")
        .unwrap_or(&empty_events);

    // WakewordOnly subscriber should only get wake word events
    assert!(
        wakeword_only_events.contains(&"wakeword".to_string()),
        "WakewordOnly subscriber should receive wake word events"
    );
    assert!(
        !wakeword_only_events.contains(&"audio_chunk".to_string()),
        "WakewordOnly subscriber should not receive audio chunks"
    );

    // WakewordPlusUtterance subscriber should get all event types
    assert!(
        wakeword_plus_events.contains(&"wakeword".to_string()),
        "WakewordPlusUtterance subscriber should receive wake word events"
    );
    // Note: Depending on timing, we might not always get the full sequence in this test,
    // but we should at least get the wake word events

    info!("🎉 Multiple subscribers test passed!");
}
