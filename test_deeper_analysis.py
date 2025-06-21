#!/usr/bin/env python3

import numpy as np
import openwakeword
from openwakeword.model import Model

# Initialize OpenWakeWord with hey mycroft model using TFLite
print("Initializing OpenWakeWord...")
oww = Model(wakeword_models=["hey mycroft"], inference_framework="tflite")

print("Model details:")
print(f"  Input size: {oww.model_inputs['hey mycroft']}")
print(f"  Output size: {oww.model_outputs['hey mycroft']}")

# Test with one chunk to see what features are extracted
chunk_size = 1280
audio_chunk = np.random.randint(-1000, 1000, chunk_size).astype(np.int16)

print(f"\nTesting with single audio chunk ({chunk_size} samples)...")
predictions = oww.predict(audio_chunk)

# Let's see what features are being extracted
print(f"\nFeature extraction details:")
print(f"  Feature buffer shape: {oww.preprocessor.feature_buffer.shape}")

# Get the features that would be fed to the model
features = oww.preprocessor.get_features(16)  # 16 feature frames as expected by the model
print(f"  Features shape for model: {features.shape}")
print(f"  Features[0] first 5 values: {features[0][:5] if len(features[0]) >= 5 else features[0]}")

# Let's also check what the melspectrogram looks like
print(f"\nMelspectrogram details:")
print(f"  Mel buffer shape: {oww.preprocessor.melspectrogram_buffer.shape}")

# Test prediction
print(f"\nPrediction: {predictions}")

# Let's trace through what happens inside the model prediction
print(f"\n" + "="*50)
print("DETAILED ANALYSIS:")
print("="*50)

# Check the model prediction function directly
print("Model prediction function for 'hey mycroft':")
model_prediction_fn = oww.model_prediction_function['hey mycroft']
print(f"  Prediction function: {model_prediction_fn}")

# Get current features being used for prediction
current_features = oww.preprocessor.get_features(16)
print(f"  Current features shape: {current_features.shape}")
print(f"  Current features dtype: {current_features.dtype}")

# Try calling the model directly
try:
    direct_prediction = model_prediction_fn(current_features)
    print(f"  Direct model output: {direct_prediction}")
    print(f"  Direct model output shape: {np.array(direct_prediction).shape}")
except Exception as e:
    print(f"  Error calling model directly: {e}")

# Compare with our expected input size
print(f"\n" + "="*50)
print("COMPARISON WITH OUR RUST IMPLEMENTATION:")
print("="*50)
print(f"OpenWakeWord expects: {oww.model_inputs['hey mycroft']} feature frames")
print(f"Our Rust code feeds: 6144 raw values (64 embeddings × 96 features)")
print(f"This suggests we're using DIFFERENT models!")

# Let's check what wakeword models are available
all_models = openwakeword.get_pretrained_model_paths('tflite')
print(f"\nAll available models:")
for model in all_models:
    print(f"  {model}") 