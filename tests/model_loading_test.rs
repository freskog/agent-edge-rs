//! Simple test to verify model loading works correctly

use agent_edge_rs::detection::pipeline::{DetectionPipeline, PipelineConfig};

#[test]
fn test_model_loading() {
    println!("🧪 Testing model loading...");

    let config = PipelineConfig::default();
    println!("Config: {:?}", config);

    match DetectionPipeline::new(config) {
        Ok(_pipeline) => {
            println!("✅ Pipeline initialized successfully");
        }
        Err(e) => {
            println!("❌ Pipeline initialization failed: {}", e);
            panic!("Model loading failed: {}", e);
        }
    }
}

#[test]
fn test_single_chunk_processing() {
    println!("🧪 Testing single chunk processing...");

    let config = PipelineConfig::default();
    let mut pipeline = DetectionPipeline::new(config).expect("Pipeline should initialize");

    // Create a test audio chunk (1280 samples of silence)
    let test_chunk = [0.0f32; 1280];

    match pipeline.process_audio_chunk(&test_chunk) {
        Ok(detection) => {
            println!("✅ Chunk processed successfully");
            println!("   Confidence: {:.3}", detection.confidence);
            println!("   Detected: {}", detection.detected);
        }
        Err(e) => {
            println!("❌ Chunk processing failed: {}", e);
            panic!("Chunk processing failed: {}", e);
        }
    }
}
