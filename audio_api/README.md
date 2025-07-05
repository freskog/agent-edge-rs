# Audio API

Simple gRPC-based audio capture and playback service with stream-based audio management.

## Audio Format

All audio uses the following fixed format:
- **Sample Rate**: 16kHz
- **Channels**: 1 (mono)
- **Format**: 32-bit float (f32)
- **Chunk Size**: 1280 samples (80ms per chunk)

## API Methods

### SubscribeAudio
Subscribe to audio capture stream.
- **Request**: `SubscribeRequest` (empty)
- **Response**: Stream of `AudioChunk` messages

### PlayAudio
Play audio stream with stream ID.
- **Request**: Stream of `PlayAudioRequest` messages (chunks + end marker)
- **Response**: `PlayResponse` when complete

### EndAudioStream
End an audio stream explicitly.
- **Request**: `EndStreamRequest` with stream ID
- **Response**: `EndStreamResponse` with playback stats

### AbortPlayback
Abort playback by stream ID.
- **Request**: `AbortRequest` with stream ID
- **Response**: `AbortResponse`

## Stream Management

- **Stream ID**: Each audio stream has a unique identifier
- **Real-time streaming**: Send chunks as they're generated
- **Explicit end**: Send end marker when stream is complete
- **Targeted abort**: Abort specific streams by ID

## Example Usage

```rust
use tokio_stream::StreamExt;
use futures::stream;

// Subscribe to audio capture
let mut capture_stream = client.subscribe_audio(SubscribeRequest {}).await?;
while let Some(chunk) = capture_stream.next().await {
    // Process 1280 samples of audio
    let samples = chunk?.samples;
    // ...
}

// Play TTS response with real-time streaming
let stream_id = "tts_response_123";
let mut play_stream = client.play_audio().await?;

// Send audio chunks as they're generated
for chunk in tts_generator.generate_stream(text) {
    play_stream.send(PlayAudioRequest {
        stream_id: stream_id.to_string(),
        data: Some(play_audio_request::Data::Chunk(AudioChunk {
            samples: chunk,
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        }))
    }).await?;
}

// End the stream explicitly
play_stream.send(PlayAudioRequest {
    stream_id: stream_id.to_string(),
    data: Some(play_audio_request::Data::EndStream(true))
}).await?;

// Get completion response
let response = play_stream.await?;
println!("Playback completed: {}", response.into_inner().message);

// If user interrupts, abort the stream
client.abort_playback(AbortRequest {
    stream_id: stream_id.to_string()
}).await?;
```

## Protocol Buffer

See `proto/audio.proto` for the complete service definition. 