//! Stream-based speech processing pipeline
//!
//! This module implements a streaming pipeline that handles:
//! - Voice activity detection (VAD)
//! - Wakeword detection
//! - Speech collection
//! - Speech-to-text conversion

use crate::detection::pipeline::DetectionPipeline;
use crate::error::Result;
use crate::stt::STT;
use crate::AudioChunk;
use futures_util::stream::{Stream, StreamExt};
use std::pin::Pin;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Events that can occur in the speech pipeline
#[derive(Debug)]
pub enum StreamEvent {
    /// Wakeword was detected with given confidence
    WakewordDetected(f32),
    /// Pipeline is actively listening after wakeword
    WakewordActive,
    /// No wakeword detected in current audio
    NoWakeword,
    /// Speech collected after wakeword
    Speech(Vec<AudioChunk>),
    /// End of speech detected
    EndOfSpeech,
    /// Final transcript from STT
    Transcript(String),
}

/// State for wakeword detection stage
#[derive(Default)]
struct WakewordState {
    detected: bool,
    confidence: f32,
    last_detection: Option<Instant>,
    audio_buffer: Vec<AudioChunk>,
}

/// State for speech collection stage
#[derive(Default)]
struct SpeechState {
    collecting: bool,
    buffer: Vec<AudioChunk>,
    last_speech: Option<Instant>,
    silence_duration: Duration,
}

impl SpeechState {
    /// Check if we've reached end of speech based on silence duration
    fn detect_end_of_speech(&self) -> bool {
        if let Some(last_speech) = self.last_speech {
            last_speech.elapsed() > self.silence_duration
        } else {
            false
        }
    }
}

/// Configuration for the speech pipeline
#[derive(Debug, Clone)]
pub struct SpeechPipelineConfig {
    /// How long to wait after last speech before ending collection
    pub end_of_speech_timeout: Duration,
    /// Maximum duration to collect speech for
    pub max_speech_duration: Duration,
}

impl Default for SpeechPipelineConfig {
    fn default() -> Self {
        Self {
            end_of_speech_timeout: Duration::from_millis(1000),
            max_speech_duration: Duration::from_secs(30),
        }
    }
}

/// Stream-based speech processing pipeline
pub struct SpeechPipeline<S>
where
    S: STT + Send + Sync + 'static,
{
    config: SpeechPipelineConfig,
    stt: std::sync::Arc<S>,
    wakeword: DetectionPipeline,
    event_tx: mpsc::Sender<StreamEvent>,
}

impl<S> SpeechPipeline<S>
where
    S: STT + Send + Sync + 'static,
{
    /// Create a new speech pipeline
    pub fn new(
        config: SpeechPipelineConfig,
        stt: std::sync::Arc<S>,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            stt,
            wakeword: DetectionPipeline::new(Default::default())?,
            event_tx,
        })
    }

    /// Process a stream of audio chunks
    pub async fn process_stream<St>(&mut self, stream: St) -> Result<()>
    where
        St: Stream<Item = AudioChunk> + Send + 'static,
    {
        // Create wakeword detection stream
        let wakeword_stream = stream.scan(WakewordState::default(), |state, chunk| {
            let mut this = self.clone();
            async move {
                if !state.detected {
                    match this.wakeword.process_audio_chunk(&chunk.samples_f32) {
                        Ok(detection) if detection.detected => {
                            state.detected = true;
                            state.confidence = detection.confidence;
                            state.last_detection = Some(Instant::now());
                            Some(StreamEvent::WakewordDetected(detection.confidence))
                        }
                        Ok(_) => Some(StreamEvent::NoWakeword),
                        Err(e) => {
                            log::error!("Wakeword error: {}", e);
                            Some(StreamEvent::NoWakeword)
                        }
                    }
                } else {
                    Some(StreamEvent::WakewordActive)
                }
            }
        });

        // Create speech collection stream
        let speech_stream = wakeword_stream.scan(SpeechState::default(), |state, event| {
            let config = self.config.clone();
            async move {
                match event {
                    StreamEvent::WakewordDetected(conf) => {
                        state.collecting = true;
                        state.buffer.clear();
                        state.last_speech = Some(Instant::now());
                        state.silence_duration = config.end_of_speech_timeout;
                        Some(StreamEvent::WakewordDetected(conf))
                    }
                    StreamEvent::WakewordActive if state.collecting => {
                        state.buffer.push(chunk);
                        if state.detect_end_of_speech() {
                            state.collecting = false;
                            Some(StreamEvent::Speech(std::mem::take(&mut state.buffer)))
                        } else {
                            Some(StreamEvent::WakewordActive)
                        }
                    }
                    _ => Some(event),
                }
            }
        });

        // Create STT stream
        let transcript_stream = speech_stream.filter_map(|event| {
            let stt = self.stt.clone();
            let event_tx = self.event_tx.clone();
            async move {
                match event {
                    StreamEvent::Speech(chunks) => match stt.process_chunks(chunks).await {
                        Ok(transcript) => {
                            let _ = event_tx
                                .send(StreamEvent::Transcript(transcript.clone()))
                                .await;
                            Some(StreamEvent::Transcript(transcript))
                        }
                        Err(e) => {
                            log::error!("STT error: {}", e);
                            None
                        }
                    },
                    _ => None,
                }
            }
        });

        // Pin and process the stream
        tokio::pin!(transcript_stream);
        while let Some(_) = transcript_stream.next().await {}

        Ok(())
    }
}
