# Barge-In Audio Issue - Root Causes and Fixes

## Problems Found

### Problem 1: Server Blocking on Completion (CRITICAL)
Audio continues playing for ~27 seconds after barge-in, and the Stop message isn't processed until audio finishes naturally.

**Root Cause**: Server blocks in `end_stream_and_wait()` and can't read new TCP messages (like Stop) during this 27-second wait.

### Problem 2: Early PlaybackComplete Signals
Server sends `PlaybackComplete` before audio actually finishes playing, causing the client fiber to exit while audio continues in the background.

**Root Cause**: Completion check only looked at playback buffer, not the command queue containing 19+ unprocessed audio chunks.

### Problem 3: Wrong Message Type (CLIENT SIDE)
In some scenarios, the Scala client was sending **`EndOfStream`** when it should send **`Stop`** for barge-in.

### What's Happening in Your Logs
```
timestamp=2025-12-25T11:31:37.268252762Z message="🔥 Interrupting current response for barge-in"
timestamp=2025-12-25T11:31:37.268522262Z message="🔥 Sending EndOfStream to audio server"
                                                              ^^^^^^^^^
                                                              WRONG MESSAGE!
```

## Message Types Explained

### `Stop` - For Immediate Interruption (Barge-In)
**Binary Format:**
- Message Type: `2` (ProducerMessageType::Stop)
- Payload: `[timestamp: u64]`

**Behavior:**
1. Immediately drains all pending audio chunks from the queue (never adds them to buffer)
2. Clears the audio playback buffer
3. Returns instantly
4. Audio stops within ~20-50ms

**Use Cases:**
- ✅ User interrupts the assistant (barge-in)
- ✅ Emergency stop
- ✅ Cancel current response

### `EndOfStream` - For Graceful Completion
**Binary Format:**
- Message Type: `3` (ProducerMessageType::EndOfStream)
- Payload: `[timestamp: u64]`

**Behavior:**
1. Waits for all buffered audio to finish playing naturally
2. Sends `PlaybackComplete` message when done
3. Blocks until playback completes (can be minutes if lots of audio buffered!)

**Use Cases:**
- ✅ Normal end of assistant response
- ✅ When you want all audio to play completely
- ❌ NOT for barge-in/interruption

## The Fix - Client Side

Your Scala client needs to change this:

### Before (WRONG)
```scala
// In your barge-in handler
def interruptResponse(): Task[Unit] =
  for {
    _ <- ZIO.logInfo("🔥 Interrupting current response for barge-in")
    _ <- ZIO.logInfo("🔥 Sending EndOfStream to audio server")  // WRONG!
    _ <- sendMessage(ProducerMessage.EndOfStream(timestamp))    // WRONG!
  } yield ()
```

### After (CORRECT)
```scala
// In your barge-in handler
def interruptResponse(): Task[Unit] =
  for {
    _ <- ZIO.logInfo("🔥 Interrupting current response for barge-in")
    _ <- ZIO.logInfo("🔥 Sending Stop to audio server")         // CORRECT!
    _ <- sendMessage(ProducerMessage.Stop(timestamp))           // CORRECT!
  } yield ()
```

## Expected Behavior After Fix

### Before Fix
```
User: "Hey Mycroft, tell me a story"
Assistant: "Here's a brief story from the search results: A story is a chain of..."
User: [interrupts] "Hey Mycroft..."
[1-2 MINUTES OF AUDIO CONTINUES PLAYING]
Assistant: "...events that begins at one place and ends at another..."
[Finally stops]
```

### After Fix
```
User: "Hey Mycroft, tell me a story"
Assistant: "Here's a brief story from the search results: A story is a chain of..."
User: [interrupts] "Hey Mycroft..."
[AUDIO STOPS WITHIN 20-50ms]
Assistant: [immediately responds to new query]
```

## Server-Side Critical Fixes (Already Applied)

### The Blocking Bug (MAIN ISSUE)

**Problem**: The server was **blocking while waiting for playback completion**, unable to read new messages like `Stop` during this time:

```
12:52:22 - EndOfStream received → calls sink.end_stream_and_wait() [BLOCKS HERE]
12:52:28 - Client sends Stop message (sits in TCP buffer, can't be read!)
12:52:49 - end_stream_and_wait() returns (27 seconds later!)
12:52:49 - Server finally reads Stop message (but audio already finished naturally)
```

This is why your barge-in didn't work - the server was stuck waiting for playback and couldn't receive the Stop command!

**Fix**: Changed to **non-blocking completion monitoring**:
- `EndOfStream` now calls `sink.end_stream()` (non-blocking)
- Returns a receiver to monitor completion
- Message loop continues, can receive `Stop` at any time
- If `Stop` arrives, cancels pending completion and aborts immediately

### Additional Critical Bugs Fixed

I've also fixed other bugs that were causing early `PlaybackComplete` signals:

### Bug 1: Completion Checked Only on Timeout
**Problem**: Completion signals were only checked when the command queue was empty (timeout), not after processing each chunk. With 20+ chunks queued, this meant waiting for all chunks to be processed before checking if playback was complete.

**Fix**: Now checks after every chunk AND on timeout.

### Bug 2: Only Checked Playback Buffer, Not Command Queue
**Problem**: The completion check only looked at the playback buffer size, ignoring chunks still queued in the command channel. This caused premature `PlaybackComplete` signals:
1. Client sends 20 audio chunks (fills command queue)
2. Client sends `EndOfStream`
3. Audio thread processes 1 chunk, buffer is small
4. Completion check sees small buffer → sends `PlaybackComplete` ✅ **BUT 19 CHUNKS STILL IN QUEUE!**
5. Client thinks playback is done, but audio plays for 17+ more seconds

**Fix**: Now checks BOTH:
- Command queue is empty (`command_rx.is_empty()`)
- Playback buffer is nearly drained (< 20ms)

### Additional Improvements
1. **Priority Abort Handling**: When `Stop` is received, drains all pending audio chunks from the queue
2. **Reduced Queue Size**: Changed from 100 slots to 20 slots (limits max buffering to ~20 seconds)
3. **Tighter Completion Threshold**: Changed from 100ms to 20ms for faster response

These changes fix the premature `PlaybackComplete` issue, but you **still need to use the correct message type** (`Stop` for barge-in, not `EndOfStream`).

## Testing the Fix

1. Update your Scala client to send `Stop` instead of `EndOfStream` for barge-in
2. Rebuild and run your client
3. Test barge-in scenario:
   ```
   User: "Hey Mycroft, tell me a long story"
   [Wait 2 seconds]
   User: "Hey Mycroft, stop"
   ```
4. Audio should stop within 20-50ms, not minutes

## Protocol Summary

| Scenario | Message to Send | Behavior |
|----------|----------------|----------|
| Barge-in / Interrupt | `Stop` | Immediate stop (~20-50ms) |
| Normal end of response | `EndOfStream` | Wait for all audio to play |
| User says "stop" command | `Stop` | Immediate stop |
| TTS generation complete | `EndOfStream` | Let audio finish naturally |

## Additional Notes

- The `Stop` message now takes a timestamp parameter (it was previously a case object)
- Make sure your Scala `ProducerMessage` definitions match the current protocol
- After sending `Stop`, you can immediately start sending new audio chunks for the next response
- After sending `EndOfStream`, wait for `PlaybackComplete` before starting a new audio session

