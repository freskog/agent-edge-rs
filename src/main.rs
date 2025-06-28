use agent_edge_rs::{
    AudioChunk,
    audio_capture::{AudioCapture, AudioCaptureConfig, PlatformAudioCapture},
    detection::pipeline::{DetectionPipeline, PipelineConfig},
    error::Result,
    stt::{FireworksSTT, STTConfig},
    vad::{VADConfig, VADMode, VADType, create_vad},
};
use log;
use std::env;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
enum StreamGate {
    Closed,
    Open,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with environment variable support (newer env_logger API)
    env_logger::init();

    log::info!("Initializing agent-edge-rs with streaming pipeline architecture");

    // Initialize STT
    let api_key = env::var("FIREWORKS_API_KEY").map_err(|_| {
        agent_edge_rs::error::EdgeError::InvalidInput(
            "FIREWORKS_API_KEY environment variable not set".to_string(),
        )
    })?;
    let stt_config = STTConfig::default();
    let stt = Arc::new(FireworksSTT::with_config(api_key, stt_config));
    log::info!("STT initialized");

    // Create streaming channels
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioChunk>(100);
    let (stt_tx, _) = broadcast::channel::<AudioChunk>(100); // Create a broadcast channel for STT
    let (wakeword_control_tx, mut wakeword_control_rx) = mpsc::channel::<StreamGate>(10);
    let (stt_control_tx, mut stt_control_rx) = mpsc::channel::<StreamGate>(10);

    // Spawn Pipeline 1: Wakeword Detection & Streaming Control
    let _wakeword_handle = {
        let stt_control_tx = stt_control_tx.clone();
        let stt_tx_clone = stt_tx.clone(); // Clone sender for the task
        tokio::spawn(async move {
            let mut pipeline = match DetectionPipeline::new(PipelineConfig::default()) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Failed to initialize detection pipeline: {}", e);
                    return;
                }
            };

            #[derive(Debug, PartialEq)]
            enum PipelineState {
                WaitingForWakeword,
                WaitingForSpeech,
            }

            let mut state = PipelineState::WaitingForWakeword;
            let mut stt_gate = StreamGate::Closed;
            let start_time = Instant::now();
            log::info!("Pipeline 1: Wakeword detection started");

            let mut total_chunks = 0;
            let mut processed_chunks = 0;
            let mut skipped_chunks = 0;
            let mut last_debug_time = Instant::now();

            // Track recent speech activity for immediate speech detection
            let mut recent_chunks: std::collections::VecDeque<bool> = std::collections::VecDeque::new();
            const RECENT_CHUNKS_SIZE: usize = 5; // Track last 5 chunks for immediate speech detection

            // STT state tracking
            let mut speech_timeout = std::time::Instant::now();
            let mut last_speech_time = std::time::Instant::now();
            let mut silence_start: Option<std::time::Instant> = None;
            let mut user_speaking_immediately = false; // Track if user spoke right after wakeword
            const FIRST_SPEECH_TIMEOUT_MS: u128 = 8000; // 8 seconds to wait for first speech
            const SILENCE_TIMEOUT_MS: u128 = 650; // 0.65 seconds of silence to end STT

            while let Some(audio_chunk) = audio_rx.recv().await {
                total_chunks += 1;

                // Track recent speech activity
                recent_chunks.push_back(audio_chunk.should_process);
                if recent_chunks.len() > RECENT_CHUNKS_SIZE {
                    recent_chunks.pop_front();
                }

                match state {
                    PipelineState::WaitingForWakeword => {
                        if audio_chunk.should_process {
                            processed_chunks += 1;
                            // Wakeword detection
                            match pipeline.process_audio_chunk(&audio_chunk.samples_f32) {
                                Ok(detection) => {
                                    if detection.detected {
                                        println!("🚨🎉 WAKEWORD DETECTED! 🎉🚨");
                                        println!("   Confidence: {:.3}", detection.confidence);
                                        println!("   🎤 Listening for command...");
                                        println!("");

                                        // Switch to waiting for speech state
                                        state = PipelineState::WaitingForSpeech;
                                        stt_gate = StreamGate::Open;
                                        speech_timeout = std::time::Instant::now();
                                        silence_start = None;

                                        // Check if user is speaking immediately after wakeword
                                        let recent_speech_count = recent_chunks.iter().filter(|&&is_speech| is_speech).count();
                                        let immediate_speech = recent_speech_count >= 3;
                                        
                                        if immediate_speech {
                                            log::info!("Detected immediate speech after wakeword");
                                            // User is speaking immediately, start gap detection right away
                                            user_speaking_immediately = true;
                                            silence_start = None;
                                        } else {
                                            log::info!("Detected pause after wakeword - waiting for speech");
                                            // User paused, wait longer for them to start speaking
                                            user_speaking_immediately = false;
                                        }

                                        // Open STT stream (buffered audio will be sent by the STT task)
                                        if let Err(e) = stt_control_tx.send(StreamGate::Open).await {
                                            log::error!("Failed to signal STT gate open: {}", e);
                                        }

                                        log::info!("State: WaitingForWakeword → WaitingForSpeech");
                                    }
                                }
                                Err(e) => {
                                    log::error!("Wakeword detection error: {}", e);
                                }
                            }
                        } else {
                            skipped_chunks += 1;
                        }
                    }

                    PipelineState::WaitingForSpeech => {
                        if audio_chunk.should_process {
                            // Speech detected - send to STT and reset timers
                            if let Err(_) = stt_tx_clone.send(audio_chunk.clone()) {
                                log::warn!("Failed to send audio chunk to STT");
                            } else {
                                log::debug!("✓ Speech detected - sent audio chunk to STT");
                            }
                            last_speech_time = std::time::Instant::now();
                            silence_start = None;
                        } else {
                            // No speech detected
                            if silence_start.is_none() {
                                silence_start = Some(std::time::Instant::now());
                                log::debug!("🔇 Silence started in WaitingForSpeech state");
                            }

                            // Check if we should end STT session
                            let should_end_stt = if let Some(silence_time) = silence_start {
                                let silence_duration = silence_time.elapsed().as_millis();
                                log::trace!("Silence duration: {}ms (threshold: {}ms)", silence_duration, SILENCE_TIMEOUT_MS);
                                // End if we've had silence for more than the threshold
                                silence_duration > SILENCE_TIMEOUT_MS
                            } else {
                                false
                            };

                            // Check for first speech timeout, but only if user didn't speak immediately
                            let speech_elapsed = speech_timeout.elapsed().as_millis();
                            let first_speech_timeout = !user_speaking_immediately && speech_elapsed > FIRST_SPEECH_TIMEOUT_MS;
                            
                            if first_speech_timeout {
                                log::debug!("First speech timeout: {}ms (threshold: {}ms)", speech_elapsed, FIRST_SPEECH_TIMEOUT_MS);
                            }

                            if should_end_stt || first_speech_timeout {
                                log::info!("Ending STT session - {} silence: {:?}ms, first speech timeout: {}",
                                    if should_end_stt { "prolonged" } else { "no" },
                                    silence_start.map(|s| s.elapsed().as_millis()),
                                    first_speech_timeout
                                );

                                // Stop sending audio to STT and signal it to complete
                                state = PipelineState::WaitingForWakeword;
                                stt_gate = StreamGate::Closed;
                                
                                // Signal STT to stop and complete transcription
                                if let Err(e) = stt_control_tx.send(StreamGate::Closed).await {
                                    log::error!("Failed to signal STT gate close: {}", e);
                                }
                                
                                log::info!("State: WaitingForSpeech → WaitingForWakeword");
                            }
                        }
                    }
                }

                // Debug logging every 5 seconds
                if last_debug_time.elapsed() >= Duration::from_secs(5) {
                    log::info!(
                        "Pipeline state: {:?}, {}/{} chunks processed ({} skipped by VAD)",
                        state,
                        processed_chunks,
                        total_chunks,
                        skipped_chunks
                    );
                    total_chunks = 0;
                    processed_chunks = 0;
                    skipped_chunks = 0;
                    last_debug_time = Instant::now();
                }

                // Listen for gate control signals (non-blocking)
                while let Ok(gate_signal) = wakeword_control_rx.try_recv() {
                    match gate_signal {
                        StreamGate::Open => log::debug!("STT gate opened"),
                        StreamGate::Closed => {
                            log::info!("STT gate closed, returning to wakeword detection");
                            // Reset LED back to listening mode when STT completes
                            pipeline.reset_led_only();
                            // Ensure we're back in wakeword waiting state
                            state = PipelineState::WaitingForWakeword;
                            stt_gate = StreamGate::Closed;
                        }
                    }
                }
            }

            log::info!("Pipeline 1: Wakeword detection ended");
        })
    };

    // Spawn Pipeline 2: STT Task Spawner
    let _stt_handle = {
        let wakeword_control_tx = wakeword_control_tx.clone();
        let stt_tx_for_spawner = stt_tx.clone();
        tokio::spawn(async move {
            let mut stt_task_handle: Option<tokio::task::JoinHandle<()>> = None;

            log::info!("Pipeline 2: STT Task Spawner started");

            while let Some(gate_signal) = stt_control_rx.recv().await {
                match gate_signal {
                    StreamGate::Open => {
                        if let Some(handle) = stt_task_handle.take() {
                            log::warn!(
                                "STT gate opened, but a task is already running. Aborting old task."
                            );
                            handle.abort();
                        }

                        log::info!("Spawning new STT transcription task...");

                        let stt_receiver = stt_tx_for_spawner.subscribe();
                        let stt_clone = Arc::clone(&stt);
                        let control_tx_clone = wakeword_control_tx.clone();

                        let handle = tokio::spawn(async move {
                            println!("🎤 Listening for command...");

                            // Await the final transcript directly
                            match stt_clone.transcribe_stream(stt_receiver).await {
                                Ok(transcript) if !transcript.is_empty() => {
                                    println!("🗣️  Final Transcript: \"{}\"", transcript);
                                }
                                Ok(_) => {
                                    log::info!("Received empty transcript.");
                                }
                                Err(e) => {
                                    log::error!("STT task failed: {}", e);
                                    println!(
                                        "❌ Speech recognition failed. Say 'hey mycroft' to try again."
                                    );
                                }
                            }

                            // Signal that this task is done
                            println!("   Say 'hey mycroft' again or press Ctrl+C to stop...");
                            if let Err(e) = control_tx_clone.send(StreamGate::Closed).await {
                                log::error!("Failed to signal STT gate close: {}", e);
                            }
                        });
                        stt_task_handle = Some(handle);
                    }
                    StreamGate::Closed => {
                        log::info!("Received 'close' signal for STT task.");
                        // The STT task will complete naturally when the audio channel has no more data
                        // and will signal back via control_tx_clone when done
                    }
                }
            }

            log::info!("Pipeline 2: STT Task Spawner ended");
        })
    };

    // Main audio capture loop with VAD processing (keeps VAD in main thread)
    let config = AudioCaptureConfig::default();
    match PlatformAudioCapture::new(config) {
        Ok(mut audio_capture) => {
            if let Err(e) = audio_capture.start() {
                log::error!("❌ MICROPHONE START FAILED! Error: {}", e);
                return Err(agent_edge_rs::error::EdgeError::Audio(e.to_string()));
            }

            // Initialize VAD in main thread with environment variable control
            let vad_type = match std::env::var("VAD_TYPE").as_deref() {
                Ok("silero") => VADType::Silero,
                Ok("webrtc") => VADType::WebRTC,
                _ => {
                    // Default to WebRTC for better performance on Pi
                    log::info!("Using WebRTC VAD (set VAD_TYPE=silero for Silero VAD)");
                    VADType::WebRTC
                }
            };

            // VAD sensitivity tuning via environment variables
            let speech_frames = std::env::var("VAD_SPEECH_FRAMES")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(match vad_type {
                    VADType::WebRTC => 12, // Very conservative for WebRTC (ignores distant chatter)
                    VADType::Silero => 6,  // More sensitive for Silero (better for wakewords)
                });

            let silence_frames = std::env::var("VAD_SILENCE_FRAMES")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(match vad_type {
                    VADType::WebRTC => 3, // Quick silence detection
                    VADType::Silero => 8, // Longer silence for Silero stability
                });

            log::info!(
                "VAD settings: speech_frames={}, silence_frames={} (type: {:?})",
                speech_frames,
                silence_frames,
                vad_type
            );

            let vad_config = VADConfig {
                vad_type,
                mode: VADMode::VeryAggressive, // Most aggressive mode to reduce false positives
                speech_trigger_frames: speech_frames,
                silence_stop_frames: silence_frames,
                ..VADConfig::default()
            };

            let mut vad = create_vad(vad_config)?;
            log::info!("VAD initialized (type: {:?})", vad_type);

            log::info!("Microphone initialized");
            log::info!("Starting audio capture loop");

            let mut chunk_count = 0;
            let mut last_log_time = Instant::now();
            let start_time = Instant::now();

            loop {
                match audio_capture.read_chunk() {
                    Ok(audio_chunk_i16) => {
                        chunk_count += 1;

                        // Log progress every 5 seconds
                        if last_log_time.elapsed() >= Duration::from_secs(5) {
                            log::info!("Audio capture: {} chunks in last 5s", chunk_count);
                            chunk_count = 0;
                            last_log_time = Instant::now();
                        }

                        // Convert to f32 for ML processing
                        let audio_chunk_f32: Vec<f32> = audio_chunk_i16
                            .iter()
                            .map(|&x| x as f32 / 32768.0)
                            .collect();

                        // VAD processing in main thread
                        let should_process = match vad.should_process_audio(&audio_chunk_i16) {
                            Ok(result) => result,
                            Err(e) => {
                                log::error!("VAD error: {}", e);
                                false
                            }
                        };

                        let audio_chunk = AudioChunk {
                            samples_i16: audio_chunk_i16,
                            samples_f32: audio_chunk_f32,
                            timestamp: Instant::now(),
                            should_process,
                        };

                        // Send to Pipeline 1 (non-blocking)
                        if let Err(e) = audio_tx.try_send(audio_chunk) {
                            log::warn!("Audio pipeline full, dropping chunk: {}", e);
                        }

                        // Brief sleep to prevent CPU spinning
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(_) => {
                        // No audio available yet
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        }
        Err(e) => {
            log::error!("❌ MICROPHONE ACCESS FAILED! Error: {}", e);
            return Err(agent_edge_rs::error::EdgeError::Audio(e.to_string()));
        }
    }
}
