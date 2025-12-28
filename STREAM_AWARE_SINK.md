# Stream-Aware Audio Sink Implementation

## Problem

When switching between audio streams (e.g., barge-in during TTS), calling `abort()` on the audio sink:
1. **Resets hardware state** → causes audio initialization delay
2. **Clips the first chunk** of new audio → user hears cutoff
3. **Requires timing coordination** between producer and sink

## Solution: Stream-Aware Sink

The audio sink now tracks which stream is currently playing and can **switch streams atomically** without hardware reset.

### Architecture

```
Producer Thread                Audio Sink Thread
───────────────                ─────────────────
                               current_stream_id = 0
                               
Play{stream_id: 100} ────────→ New stream detected!
                               current_stream_id = 100
                               Playing stream 100...
                               
Play{stream_id: 200} ────────→ Stream switch!
                               - Drop all stream 100 chunks from:
                                 * Command queue
                                 * Playback buffer
                               - Switch: current_stream_id = 200
                               - Hardware stays WARM 🔥
                               Playing stream 200 immediately!
```

### Key Changes

#### 1. `AudioCommand::WriteChunk` now carries `stream_id`

```rust
enum AudioCommand {
    WriteChunk { data: Vec<u8>, stream_id: u64 },
    // ...
}
```

#### 2. `AudioSink::write_chunk()` accepts `stream_id`

```rust
pub fn write_chunk(&self, s16le_data: Vec<u8>, stream_id: u64) -> Result<(), AudioError>
```

#### 3. Audio thread tracks current stream and drops old chunks

```rust
let mut current_stream_id: u64 = 0;

match command {
    AudioCommand::WriteChunk { data, stream_id } => {
        if stream_id != current_stream_id {
            // NEW STREAM!
            // 1. Drain command queue of old chunks
            // 2. Clear playback buffer
            // 3. Switch to new stream
            // 4. NO hardware reset!
            current_stream_id = stream_id;
        }
        // Add chunk to buffer
    }
}
```

#### 4. Producer no longer calls `abort()` on stream switch

```rust
// OLD:
if stream_id != current_stream_id {
    sink.abort()?;  // ❌ Causes hardware reset & clipping
    current_stream_id = stream_id;
}
sink.write_chunk(data)?;

// NEW:
if stream_id != current_stream_id {
    // Just update tracking - sink handles the switch!
    current_stream_id = stream_id;
}
sink.write_chunk(data, stream_id)?;  // ✅ Atomic stream switch
```

## Benefits

✅ **No audio clipping** - Hardware stays warm, no initialization delay  
✅ **Instant stream switching** - Old audio dropped at buffer level  
✅ **Simpler coordination** - No need for timing between threads  
✅ **Clean semantics** - `abort()` only for true shutdown scenarios  
✅ **Race-free** - Stream switching is atomic in audio thread  

## Message Flow (Barge-In Scenario)

```
T+0.0s: User says wakeword
        └─ Detection thread sends barge-in signal
        └─ Wakeword sent to client
        
T+0.5s: Client generates new TTS (stream_id: 200)
        └─ First chunk arrives at producer
        
T+0.5s: Producer sees new stream_id
        └─ Drain stale barge-in signals
        └─ Send chunk to sink with stream_id: 200
        
T+0.5s: Audio sink receives chunk
        └─ Detects stream_id changed (100 → 200)
        └─ Drops all stream 100 chunks from queue & buffer
        └─ Clears playback buffer
        └─ Plays stream 200 immediately
        └─ Hardware never reset! 🎉
```

## Testing

Run the audio server and trigger barge-in:
- Old behavior: Hear first chunk clipped/cut off
- New behavior: Clean instant switch, no clipping

Logs to look for:
```
🔄 Stream switch: 100 → 200 (dropping old audio)
🗑️  Dropped N old chunks from stream 100
```

You should **NOT** see:
```
🗑️  Drained 1 buffered audio chunks during abort  ← This was the problem!
```

## Related Files

- `src/audio_sink.rs` - Stream-aware audio thread
- `src/producer_server.rs` - Simplified stream switching
- `src/protocol.rs` - Stream ID in Play messages

