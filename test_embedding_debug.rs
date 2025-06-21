use agent_edge_rs::{
    detection::pipeline::{DetectionPipeline, OpenWakeWordConfig},
    error::Result,
};

fn main() -> Result<()> {
    env_logger::init();

    println!("🔍 DEBUGGING EMBEDDING ISSUE WITH FAKE AUDIO...");

    // Initialize the pipeline
    let config = OpenWakeWordConfig::default();
    let mut pipeline = DetectionPipeline::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        "models/hey_mycroft_v0.1.tflite",
        config,
    )?;

    // Generate fake audio data that changes over time
    for i in 0..10 {
        println!("\n=== Testing chunk {} ===", i);

        // Create 1280 samples of audio with some variation
        let mut fake_audio = vec![0i16; 1280];
        for (j, sample) in fake_audio.iter_mut().enumerate() {
            // Add some patterns that change over time
            *sample = ((i * 100 + j / 10) % 32000) as i16;
        }

        let detection = pipeline.process_audio_chunk(&fake_audio)?;
        println!("Chunk {} confidence: {:.4}", i, detection.confidence);

        if i >= 5 {
            // Only run a few iterations to see the pattern
            break;
        }
    }

    Ok(())
}
