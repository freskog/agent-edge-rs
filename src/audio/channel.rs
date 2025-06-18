use crate::error::{EdgeError, Result};

/// Extracts a specific channel from interleaved multi-channel audio
#[derive(Clone)]
pub struct ChannelExtractor {
    target_channel: usize,
    total_channels: usize,
}

impl ChannelExtractor {
    pub fn new(target_channel: usize, total_channels: usize) -> Result<Self> {
        if target_channel >= total_channels {
            return Err(EdgeError::Audio(format!(
                "Target channel {} is out of range for {} channels",
                target_channel, total_channels
            )));
        }
        
        Ok(Self {
            target_channel,
            total_channels,
        })
    }
    
    /// Extract the target channel from interleaved audio samples
    pub fn extract_channel(&self, interleaved_samples: &[f32]) -> Vec<f32> {
        if self.total_channels == 1 {
            // Mono audio - just return all samples
            interleaved_samples.to_vec()
        } else {
            // Multi-channel audio - extract the target channel
            interleaved_samples
                .iter()
                .enumerate()
                .filter_map(|(i, &sample)| {
                    if i % self.total_channels == self.target_channel {
                        Some(sample)
                    } else {
                        None
                    }
                })
                .collect()
        }
    }
} 