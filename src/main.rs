use agent_edge_rs::{
    audio_capture::{AudioCaptureConfig, CpalAudioCapture},
    detection::pipeline::{DetectionPipeline, PipelineConfig},
    error::Result,
    llm::tools::ToolManager,
    speech_producer::{speech_stream, SpeechHub},
    stt::{FireworksSTT, STTConfig},
    vad::{ChunkSize, VADConfig, VADSampleRate},
};
use futures_util::StreamExt;
use log;
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pipeline1Mode {
    Normal,     // Audio->VAD->WW->EOS->STT
    AnswerUser, // Audio->VAD->EOS->STT (no wakeword detection)
}

// Pipeline 1 outputs - completely independent
#[derive(Debug, Clone)]
enum Pipeline1Output {
    TranscriptReady(String),
}

// Pipeline 2 outputs - completely independent
#[derive(Debug, Clone)]
enum Pipeline2Output {
    TaskCompleted(String),
    RequestUserInput(String), // Tool wants to ask user something
}

// Orchestrator commands to pipelines
#[derive(Debug, Clone)]
enum OrchestratorCommand {
    SetPipeline1Mode(Pipeline1Mode),
    CancelPipeline2,
    ProcessTranscript(String),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with environment variable support
    env_logger::init();

    log::info!("Initializing agent-edge-rs with SpeechHub architecture");

    // Initialize STT
    let api_key = env::var("FIREWORKS_API_KEY").map_err(|_| {
        agent_edge_rs::error::EdgeError::InvalidInput(
            "FIREWORKS_API_KEY environment variable not set".to_string(),
        )
    })?;
    let stt_config = STTConfig::default();
    let _stt = Arc::new(FireworksSTT::with_config(api_key, stt_config));
    log::info!("STT initialized");

    // Initialize VAD configuration
    let vad_config = VADConfig {
        sample_rate: VADSampleRate::Rate16kHz,
        chunk_size: ChunkSize::Small, // 512 samples = 32ms at 16kHz
        threshold: 0.5,
        speech_trigger_chunks: 2, // 64ms of speech to trigger
        silence_stop_chunks: 8,   // 256ms of silence to stop
    };

    log::info!(
        "VAD settings: chunk_size={:?}, threshold={}, speech_trigger={}, silence_stop={} (type: Silero)",
        vad_config.chunk_size,
        vad_config.threshold,
        vad_config.speech_trigger_chunks,
        vad_config.silence_stop_chunks
    );

    // Create orchestrator communication channels
    let (pipeline1_output_tx, mut pipeline1_output_rx) = mpsc::channel::<Pipeline1Output>(10);
    let (pipeline2_output_tx, mut pipeline2_output_rx) = mpsc::channel::<Pipeline2Output>(10);
    let (pipeline1_cmd_tx, _pipeline1_cmd_rx) = mpsc::channel::<Pipeline1Mode>(10);
    let (pipeline2_cmd_tx, mut pipeline2_cmd_rx) = mpsc::channel::<OrchestratorCommand>(10);

    // Initialize SpeechHub (single-threaded VAD processing)
    let mut speech_hub = SpeechHub::new(vad_config)?;

    // Create audio source
    let audio_config = AudioCaptureConfig::default();
    let audio_source = CpalAudioCapture::new(audio_config)
        .map_err(|e| agent_edge_rs::error::EdgeError::Audio(e.to_string()))?;

    // Subscribe to speech hub (multiple subscribers get same VAD-processed data)
    let speech_rx1 = speech_hub.subscribe(); // For wakeword detection
    let speech_rx2 = speech_hub.subscribe(); // For STT (could be used later)
    let speech_rx3 = speech_hub.subscribe(); // For LED ring control (could be used later)

    log::info!(
        "SpeechHub: {} subscribers created",
        speech_hub.subscriber_count()
    );

    // Run SpeechHub (single-threaded VAD processing and broadcasting)
    let speech_hub_task = tokio::spawn(async move {
        if let Err(e) = speech_hub.run(audio_source).await {
            log::error!("SpeechHub failed: {}", e);
        }
    });

    // Pipeline 1: Wakeword Detection (subscribes to speech events)
    let _pipeline1_handle = {
        let pipeline1_output_tx = pipeline1_output_tx.clone();

        tokio::spawn(async move {
            log::info!("Pipeline 1: Wakeword detection started");

            let mut pipeline = match DetectionPipeline::new(PipelineConfig::default()) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Failed to initialize detection pipeline: {}", e);
                    return;
                }
            };

            // Create speech stream from broadcast receiver
            let mut speech_stream = Box::pin(
                speech_stream(speech_rx1)
                    .filter(|chunk| std::future::ready(chunk.should_process())),
            );

            // Rechunk for wakeword detection (1280 samples = 80ms)
            let mut wakeword_buffer = Vec::new();
            const WAKEWORD_CHUNK_SIZE: usize = 1280;

            while let Some(chunk) = speech_stream.next().await {
                wakeword_buffer.extend_from_slice(&chunk.samples_f32);

                // Process complete 80ms chunks for wakeword detection
                while wakeword_buffer.len() >= WAKEWORD_CHUNK_SIZE {
                    let wakeword_chunk: Vec<f32> =
                        wakeword_buffer.drain(..WAKEWORD_CHUNK_SIZE).collect();

                    match pipeline.process_audio_chunk(&wakeword_chunk) {
                        Ok(detection) => {
                            if detection.detected {
                                println!("🚨🎉 WAKEWORD DETECTED! 🎉🚨");
                                println!("   Confidence: {:.3}", detection.confidence);
                                println!("   🎤 Listening for command...");
                                println!("");

                                if let Err(e) = pipeline1_output_tx
                                    .send(Pipeline1Output::TranscriptReady(
                                        "test command".to_string(),
                                    ))
                                    .await
                                {
                                    log::error!("Failed to send transcript: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Wakeword detection error: {}", e);
                        }
                    }
                }
            }

            log::info!("Pipeline 1: Speech stream ended");
        })
    };

    // Pipeline 2: LLM/Tool Execution Pipeline
    let _pipeline2_handle = {
        let mode_control_tx = pipeline1_cmd_tx.clone();
        tokio::spawn(async move {
            log::info!("Pipeline 2: LLM/Tool execution started");

            let mut _tool_manager = ToolManager::new();
            let mut current_task: Option<tokio::task::JoinHandle<()>> = None;

            while let Some(command) = pipeline2_cmd_rx.recv().await {
                match command {
                    OrchestratorCommand::ProcessTranscript(transcript) => {
                        log::info!("Pipeline 2: Processing transcript: '{}'", transcript);

                        // Cancel any running task
                        if let Some(handle) = current_task.take() {
                            log::info!("Pipeline 2: Cancelling previous task for new transcript");
                            handle.abort();
                        }

                        // Spawn new LLM/tool processing task
                        let output_tx_clone = pipeline2_output_tx.clone();
                        let _mode_control_tx_clone = mode_control_tx.clone();
                        let task_handle = tokio::spawn(async move {
                            log::info!(
                                "Pipeline 2: [PLACEHOLDER] Processing '{}' with LLM",
                                transcript
                            );

                            // Example of "ask_user" tool invocation:
                            if transcript.to_lowercase().contains("ask me") {
                                log::info!("Pipeline 2: Simulating 'ask_user' tool");

                                if let Err(e) = output_tx_clone
                                    .send(Pipeline2Output::RequestUserInput(
                                        "What's your favorite color?".to_string(),
                                    ))
                                    .await
                                {
                                    log::error!("Failed to request user input: {}", e);
                                    return;
                                }

                                tokio::time::sleep(Duration::from_secs(5)).await;
                                println!("Thank you for your answer!");
                            } else {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                println!("✅ Processed: '{}'", transcript);
                            }

                            if let Err(e) = output_tx_clone
                                .send(Pipeline2Output::TaskCompleted(format!(
                                    "Completed: {}",
                                    transcript
                                )))
                                .await
                            {
                                log::error!("Failed to signal task completion: {}", e);
                            }
                        });

                        current_task = Some(task_handle);
                    }

                    OrchestratorCommand::CancelPipeline2 => {
                        log::info!("Pipeline 2: Received cancellation command");
                        if let Some(handle) = current_task.take() {
                            log::info!("Pipeline 2: Cancelling current task");
                            handle.abort();
                        }
                    }

                    OrchestratorCommand::SetPipeline1Mode(_) => {
                        log::warn!("Pipeline 2: Received SetPipeline1Mode command - ignoring");
                    }
                }
            }

            log::info!("Pipeline 2: LLM/Tool execution ended");
        })
    };

    // Optional: LED Ring Control Pipeline (demonstrates multiple subscribers)
    let _led_pipeline_handle = {
        tokio::spawn(async move {
            log::info!("LED Pipeline: Started (speech event monitoring)");

            let mut speech_stream = speech_stream(speech_rx3);

            while let Some(chunk) = speech_stream.next().await {
                match chunk.speech_event {
                    agent_edge_rs::speech_producer::SpeechEvent::StartedSpeaking => {
                        log::debug!("LED: Speech started - could turn on listening indicator");
                    }
                    agent_edge_rs::speech_producer::SpeechEvent::StoppedSpeaking => {
                        log::debug!("LED: Speech ended - could turn off listening indicator");
                    }
                    _ => {} // Speaking events don't need special LED handling
                }
            }

            log::info!("LED Pipeline: Speech stream ended");
        })
    };

    // Orchestrator: Manages coordination between pipelines
    let _orchestrator_handle = {
        tokio::spawn(async move {
            log::info!("Orchestrator: Started managing pipeline coordination");

            #[derive(Debug, PartialEq)]
            enum OrchestratorState {
                Idle,
                Pipeline2Running,
                WaitingForUserAnswer,
            }

            let mut state = OrchestratorState::Idle;

            loop {
                tokio::select! {
                    Some(p1_output) = pipeline1_output_rx.recv() => {
                        match p1_output {
                            Pipeline1Output::TranscriptReady(transcript) => {
                                log::info!("Orchestrator: Received transcript from Pipeline 1: '{}'", transcript);

                                match state {
                                    OrchestratorState::Idle => {
                                        log::info!("Orchestrator: Starting Pipeline 2 with transcript");
                                        if let Err(e) = pipeline2_cmd_tx.send(OrchestratorCommand::ProcessTranscript(transcript)).await {
                                            log::error!("Failed to send transcript to Pipeline 2: {}", e);
                                        } else {
                                            state = OrchestratorState::Pipeline2Running;
                                        }
                                    }

                                    OrchestratorState::Pipeline2Running => {
                                        log::info!("Orchestrator: Racing condition - Pipeline 1 wins, cancelling Pipeline 2");
                                        if let Err(e) = pipeline2_cmd_tx.send(OrchestratorCommand::CancelPipeline2).await {
                                            log::error!("Failed to cancel Pipeline 2: {}", e);
                                        }

                                        if let Err(e) = pipeline2_cmd_tx.send(OrchestratorCommand::ProcessTranscript(transcript)).await {
                                            log::error!("Failed to send new transcript to Pipeline 2: {}", e);
                                        }
                                    }

                                    OrchestratorState::WaitingForUserAnswer => {
                                        log::info!("Orchestrator: Received user answer, passing to Pipeline 2");
                                        if let Err(e) = pipeline2_cmd_tx.send(OrchestratorCommand::ProcessTranscript(transcript)).await {
                                            log::error!("Failed to send user answer to Pipeline 2: {}", e);
                                        }

                                        if let Err(e) = pipeline1_cmd_tx.send(Pipeline1Mode::Normal).await {
                                            log::error!("Failed to switch Pipeline 1 back to Normal mode: {}", e);
                                        }

                                        state = OrchestratorState::Pipeline2Running;
                                    }
                                }
                            }
                        }
                    }

                    Some(p2_output) = pipeline2_output_rx.recv() => {
                        match p2_output {
                            Pipeline2Output::TaskCompleted(result) => {
                                log::info!("Orchestrator: Pipeline 2 completed: {}", result);
                                state = OrchestratorState::Idle;
                                println!("   Say 'hey mycroft' again or press Ctrl+C to stop...");
                            }

                            Pipeline2Output::RequestUserInput(question) => {
                                log::info!("Orchestrator: Pipeline 2 requests user input: {}", question);
                                println!("🤔 {}", question);

                                if let Err(e) = pipeline1_cmd_tx.send(Pipeline1Mode::AnswerUser).await {
                                    log::error!("Failed to switch Pipeline 1 to AnswerUser mode: {}", e);
                                } else {
                                    state = OrchestratorState::WaitingForUserAnswer;
                                }
                            }
                        }
                    }

                    else => {
                        log::info!("Orchestrator: All channels closed, shutting down");
                        break;
                    }
                }
            }

            log::info!("Orchestrator: Coordination ended");
        })
    };

    println!(
        "🎤 SpeechHub started with {} subscribers. Say 'hey mycroft' to test wakeword detection...",
        3
    );

    // Wait for shutdown signal
    tokio::select! {
        _ = speech_hub_task => {
            log::info!("SpeechHub ended");
        }
        _ = tokio::signal::ctrl_c() => {
            log::info!("Received Ctrl+C, shutting down...");
        }
    }

    Ok(())
}
