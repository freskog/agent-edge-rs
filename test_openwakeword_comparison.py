#!/usr/bin/env python3

import numpy as np
import openwakeword
from openwakeword.model import Model

# Initialize OpenWakeWord with hey mycroft model using TFLite (same as our Rust implementation)
print("Initializing OpenWakeWord...")
oww = Model(wakeword_models=["hey mycroft"], inference_framework="tflite")

print("OpenWakeWord models loaded:")
for model_name in oww.models.keys():
    print(f"  - {model_name}")
    print(f"    Input size: {oww.model_inputs[model_name]}")
    print(f"    Output size: {oww.model_outputs[model_name]}")

# Generate some fake audio data to test with
print("\nGenerating test audio data...")
chunk_size = 1280  # 80ms at 16kHz
num_chunks = 10

# Test the streaming behavior
print("\nTesting OpenWakeWord streaming behavior:")

for i in range(num_chunks):
    # Generate fake audio chunk (1280 samples of noise)
    audio_chunk = np.random.randint(-1000, 1000, chunk_size).astype(np.int16)
    
    print(f"\n--- Chunk {i+1} ---")
    print(f"Audio chunk shape: {audio_chunk.shape}")
    
    # Feed to OpenWakeWord
    predictions = oww.predict(audio_chunk)
    
    # Check internal state
    print(f"Preprocessor accumulated_samples: {oww.preprocessor.accumulated_samples}")
    print(f"Mel buffer shape: {oww.preprocessor.melspectrogram_buffer.shape}")
    print(f"Feature buffer shape: {oww.preprocessor.feature_buffer.shape}")
    
    # Print predictions
    for model_name, score in predictions.items():
        print(f"{model_name}: {score:.6f}")
    
    # Log feature buffer changes 
    if i > 0:
        print(f"Feature buffer size changed: {prev_feature_size} -> {oww.preprocessor.feature_buffer.shape[0]}")
        if oww.preprocessor.feature_buffer.shape[0] > prev_feature_size:
            print(f"Added {oww.preprocessor.feature_buffer.shape[0] - prev_feature_size} new embeddings")
    
    prev_feature_size = oww.preprocessor.feature_buffer.shape[0]

print("\n" + "="*50)
print("ANALYSIS COMPLETE")
print("="*50) 