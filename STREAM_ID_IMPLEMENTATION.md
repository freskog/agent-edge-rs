# Stream ID Implementation

## Overview

Implemented stream identifiers to solve audio clipping and barge-in timing issues. Stream IDs provide a robust, declarative approach to managing audio sessions that eliminates race conditions and timing dependencies.

## Changes Made

### 1. Protocol Changes (`src/protocol.rs`)

**Removed:**
- `ProducerMessage::Stop` variant - barge-in now only stops server-side
- Stop message handling code

**Added:**
- `stream_id: u64` field to `ProducerMessage::Play`
- `stream_id: u64` field to `ProducerMessage::EndOfStream`

**Message Format:**
- **Play**: `[type: u8][payload_len: u32][stream_id: u64][data: bytes]`
- **EndOfStream**: `[type: u8][payload_len: u32][timestamp: u64][stream_id: u64]`

### 2. Producer Server State (`src/producer_server.rs`)

**Replaced complex state:**
```rust
// OLD:
let mut is_playing: bool;
let mut pending_completion: Option<mpsc::Receiver<()>>;
// + drain logic on connection + session start + after barge-in

// NEW:
let mut current_stream_id: u64 = 0;         // 0 = idle, >0 = playing
let mut interrupted_stream_id: u64 = 0;     // Last interrupted stream
let mut pending_completion: Option<mpsc::Receiver<()>>;
```

### 3. Play Message Handler

**Logic:**
```rust
1. If stream_id <= interrupted_stream_id → DROP (old/interrupted stream)
2. If stream_id != current_stream_id → NEW STREAM
   - Abort old stream if playing
   - Set current_stream_id = stream_id
3. Write audio to sink
```

### 4. EndOfStream Handler

**Logic:**
```rust
1. If stream_id == current_stream_id → Start completion monitoring
2. Else → IGNORE (old stream)
```

### 5. Barge-in Handler

**Logic:**
```rust
1. If current_stream_id != 0:
   - interrupted_stream_id = current_stream_id
   - current_stream_id = 0
   - Abort playback
   - Send PlaybackComplete
```

### 6. Removed Code

- **Stop message handling** - entire block deleted
- **Drain logic** - connection-time, session-start, and post-barge-in drains removed
- **`is_playing` flag** - replaced by `current_stream_id != 0` check
- **Trace logging** - cleaned up debug/trace logs added during investigation

## Benefits

### Before (Timing-Based)
❌ Race conditions with barge-in signals  
❌ Stale signals could abort new audio  
❌ Complex drain logic at multiple points  
❌ Timing-dependent behavior  
❌ First audio chunk could be clipped  

### After (Stream ID-Based)
✅ No race conditions - stream IDs are orderable  
✅ Old audio automatically dropped  
✅ No drain logic needed  
✅ Timing-independent  
✅ Network resilient (reordered packets handled)  
✅ Debuggable (trace each chunk to its stream)  

## Client Changes Required

The Scala client must be updated to:

```scala
// Generate stream ID (Unix timestamp in milliseconds)
val streamId = System.currentTimeMillis()

// Include in all Play messages
ProducerMessage.Play(audioChunk, streamId)

// Include in EndOfStream
ProducerMessage.EndOfStream(timestamp, streamId)

// Remove any Stop message sending code
```

## Architecture Simplification

The stream ID approach transforms the problem from **reactive** (drain stale signals) to **declarative** (compare IDs):

```
OLD: Time-based coordination
- Send barge-in signal
- Hope it arrives before new audio
- Drain channel at multiple points
- Complex state machine

NEW: Identity-based coordination  
- Each audio session has unique ID
- Drop audio from old sessions
- Simple comparison: if stream_id <= interrupted → drop
- 3 simple state variables
```

## Testing

Built successfully with:
```bash
cargo build --release
```

No compiler errors. All tests pass. Ready for integration testing with updated Scala client.

## Next Steps

1. Update Scala client to include `stream_id` in messages
2. Test barge-in functionality with stream IDs
3. Verify no audio clipping at start of TTS
4. (Future) Consider simplifying AudioSink architecture (remove command thread for Path B: sync design)

