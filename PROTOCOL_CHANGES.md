# Audio Protocol - JSON Format Migration

## Overview

The audio protocol has been migrated from a binary format with message type bytes to a JSON-based format that follows the zio-json sum type convention. This makes it much easier to work with from Scala clients.

## Protocol Changes

### Before (Binary Protocol)
```
[MessageType: u8][PayloadSize: u32][Binary Payload]
```

### After (JSON Protocol)
```
[PayloadSize: u32][JSON Payload]
```

## Message Format

All messages now follow the zio-json sum type convention:

**Case classes** (variants with data):
```json
{
  "VariantName": { /* variant fields */ }
}
```

**Case objects** (variants without data):
```json
"VariantName"
```

### Consumer Messages (Port 8080)

#### Subscribe
```json
{
  "Subscribe": {
    "id": "client-123"
  }
}
```

#### Connected
```json
"Connected"
```

#### Error
```json
{
  "Error": {
    "message": "Something went wrong"
  }
}
```

#### Audio
```json
{
  "Audio": {
    "data": "base64-encoded-audio-data",
    "speechDetected": true
  }
}
```

#### WakewordDetected
```json
{
  "WakewordDetected": {
    "model": "hey-jarvis"
  }
}
```

### Producer Messages (Port 8081)

#### Play
```json
{
  "Play": {
    "data": "base64-encoded-audio-data"
  }
}
```

#### Stop
```json
"Stop"
```

#### Connected
```json
"Connected"
```

#### Error
```json
{
  "Error": {
    "message": "Playback failed"
  }
}
```

## Scala Usage with zio-json

### Define Protocol
```scala
import zio.json._

sealed trait ConsumerMessage

object ConsumerMessage {
  case class Subscribe(id: String) extends ConsumerMessage
  case object Connected extends ConsumerMessage
  case class Error(message: String) extends ConsumerMessage
  case class Audio(data: String, speechDetected: Boolean) extends ConsumerMessage
  case class WakewordDetected(model: String) extends ConsumerMessage

  implicit val decoder: JsonDecoder[ConsumerMessage] = DeriveJsonDecoder.gen[ConsumerMessage]
  implicit val encoder: JsonEncoder[ConsumerMessage] = DeriveJsonEncoder.gen[ConsumerMessage]
}
```

### Parse Messages
```scala
// Easy parsing - exactly like your Fruit example!
"""{"Subscribe":{"id":"client-123"}}""".fromJson[ConsumerMessage]
// res: Either[String, ConsumerMessage] = Right(Subscribe("client-123"))

"""{"WakewordDetected":{"model":"hey-jarvis"}}""".fromJson[ConsumerMessage]  
// res: Either[String, ConsumerMessage] = Right(WakewordDetected("hey-jarvis"))

// Case objects are just strings
""""Connected"""".fromJson[ConsumerMessage]
// res: Either[String, ConsumerMessage] = Right(Connected)
```

### Send Messages
```scala
val msg = ConsumerMessage.Subscribe("my-client")
val json = msg.toJson
// json: String = {"Subscribe":{"id":"my-client"}}
```

## Benefits

1. **Type Safety**: zio-json automatically handles sum type serialization/deserialization
2. **No Message Type Bytes**: Simpler framing with just payload size
3. **Human Readable**: JSON messages are easy to debug and inspect
4. **Base64 Binary Data**: Binary audio data is safely encoded as base64 in JSON
5. **Consistent Convention**: Follows standard zio-json sum type patterns

## Migration Notes

- **Rust Side**: All serialization now uses `serde_json` with automatic base64 encoding for binary data
- **Framing**: Still uses 4-byte little-endian payload size for reliable message framing
- **Binary Data**: Audio data is base64-encoded in the JSON payload
- **Compatibility**: This is a breaking change - all clients need to migrate to JSON format

## Example Usage

See `examples/scala_client_example.scala` for a complete working example of how to connect and communicate with the audio server using the new JSON protocol.

## Testing

Run the protocol tests to verify everything is working:
```bash
cargo test protocol
```

All tests verify the JSON serialization, base64 encoding, and round-trip compatibility. 