#!/usr/bin/env python3

import tflite_runtime.interpreter as tflite

def inspect_model(model_path, model_name):
    print(f"\n{'='*50}")
    print(f"INSPECTING: {model_name}")
    print(f"File: {model_path}")
    print('='*50)
    
    try:
        # Load the TFLite model
        interpreter = tflite.Interpreter(model_path=model_path)
        interpreter.allocate_tensors()
        
        # Get input and output details
        input_details = interpreter.get_input_details()
        output_details = interpreter.get_output_details()
        
        print(f"INPUT DETAILS:")
        for i, detail in enumerate(input_details):
            print(f"  Input {i}:")
            print(f"    Name: {detail['name']}")
            print(f"    Shape: {detail['shape']}")
            print(f"    Type: {detail['dtype']}")
            
        print(f"\nOUTPUT DETAILS:")
        for i, detail in enumerate(output_details):
            print(f"  Output {i}:")
            print(f"    Name: {detail['name']}")
            print(f"    Shape: {detail['shape']}")
            print(f"    Type: {detail['dtype']}")
            
        # Calculate total input/output size
        total_input_size = 1
        for dim in input_details[0]['shape']:
            if dim > 0:  # Skip batch dimension if -1
                total_input_size *= dim
                
        total_output_size = 1  
        for dim in output_details[0]['shape']:
            if dim > 0:
                total_output_size *= dim
                
        print(f"\nTOTAL INPUT SIZE: {total_input_size}")
        print(f"TOTAL OUTPUT SIZE: {total_output_size}")
        
    except Exception as e:
        print(f"ERROR: {e}")

# Inspect our models
inspect_model("models/melspectrogram.tflite", "Melspectrogram Model")
inspect_model("models/embedding_model.tflite", "Embedding Model") 
inspect_model("models/hey_mycroft_v0.1.tflite", "Hey Mycroft Model (Ours)")

# Compare with the official OpenWakeWord model
official_model_path = "/home/vscode/.local/lib/python3.10/site-packages/openwakeword/resources/models/hey_mycroft_v0.1.tflite"
inspect_model(official_model_path, "Hey Mycroft Model (Official OpenWakeWord)")

print(f"\n{'='*50}")
print("ANALYSIS:")
print('='*50)
print("If our model expects 6144 inputs but OpenWakeWord expects 1536,")
print("then we're using incompatible models!")
print("We need to:")
print("1. Use the same model as OpenWakeWord")
print("2. Feed it 16 embedding frames (16×96=1536) instead of 64 (64×96=6144)") 