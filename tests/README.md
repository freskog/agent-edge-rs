# Test Suite for Agent Edge RS

This directory contains comprehensive tests for the Rust OpenWakeWord implementation.

## Test Structure

### 📊 Unit Tests (`cargo test --lib`)
Located in `src/` alongside the source code:
- **Pipeline Configuration**: Default values, validation
- **Model Creation**: Basic initialization without loading actual models  
- **Melspectrogram**: Config validation and sample generation

### 🔊 Audio Tests (`cargo test --test audio_tests`)
Tests for audio processing utilities:
- **Channel Extraction**: Multi-channel to mono conversion (ReSpeaker 6→1)
- **Format Conversion**: i16 to f32 sample conversion 
- **Edge Cases**: Empty audio, invalid channels
- **Real Hardware**: ReSpeaker-specific channel layout validation

### 🎯 Pipeline Integration Test (`cargo test --test pipeline_tests`)
Single comprehensive end-to-end test with real TensorFlow Lite models:
- **Model Loading**: Full pipeline initialization with real models
- **Audio Processing**: Hey Mycroft test file (real detection)
- **Silence Handling**: No false positives  
- **Chunk Validation**: Proper error handling
- **Reset Functionality**: State clearing verification
- **Debouncing**: Prevents repeated detections
- **Confidence Analysis**: Validates detection accuracy

## Running Tests

### All Tests
```bash
cargo test
```

### Individual Test Suites
```bash
cargo test --lib                    # Unit tests
cargo test --test audio_tests       # Audio processing tests  
cargo test --test pipeline_tests    # End-to-end integration test
```

### Verbose Output
```bash
cargo test test_complete_pipeline --test pipeline_tests -- --nocapture
```

## Test Data

### Audio Files
- `tests/data/hey_mycroft_test.wav`: Real "Hey Mycroft" utterance from OpenWakeWord test suite
- **Format**: 16kHz mono 16-bit WAV (0.95 seconds)
- **Purpose**: Validates end-to-end detection accuracy

### Model Requirements
Pipeline tests require these TensorFlow Lite models in `models/`:
- `melspectrogram.tflite` (80ms audio → mel features)
- `embedding_model.tflite` (mel frames → embeddings)  
- `hey_mycroft_v0.1.tflite` (embeddings → detection)

## Test Coverage

### ✅ Current Status
- **Unit tests**: 5/5 pass
- **Audio tests**: 7/7 pass
- **Pipeline integration**: 1 comprehensive test covering all functionality
- **Total**: 13/13 tests pass

### 🎯 Comprehensive Pipeline Test
The single pipeline test validates:

1. **Configuration**: Default values and validation
2. **Initialization**: Model loading and setup
3. **Silence Processing**: No false positives on quiet audio
4. **Input Validation**: Proper error handling for wrong chunk sizes
5. **Reset Functionality**: State clearing works correctly
6. **Real Audio**: End-to-end detection with Hey Mycroft test file
7. **Debouncing**: Limited detections despite multiple high-confidence chunks

## Expected Results

### Hey Mycroft Detection
When working correctly, the pipeline test should show:
```
✅ 6a. Loaded test audio: 15232 samples (0.95s)
📏 Audio length: original 0.95s → padded 2.95s
✅ 6b. Processed 37 chunks
📊 Detection Results:
   - Total chunks: 37
   - Detections: 1
   - Max confidence: 1.0000
   - Average confidence: 0.1364
✅ 6c. Hey Mycroft audio processing validated
✅ 7. Debouncing appears to be working (limited detections)
🎉 All pipeline tests passed! System is working correctly.
```

### Performance Characteristics
- **Latency**: ~1.3 seconds (due to required temporal context)
- **Detection Accuracy**: Peak confidence 1.0 matches OpenWakeWord expectations
- **Memory**: Fixed-size rolling windows (~16KB total)
- **Robustness**: Handles edge cases and validates all inputs properly 