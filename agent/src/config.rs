use secrecy::{ExposeSecret, SecretBox};
use std::env;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingEnvVar(String),
    #[error("Invalid API key format for {service}: {reason}")]
    InvalidKeyFormat { service: String, reason: String },
    #[error("Environment error: {0}")]
    EnvError(#[from] env::VarError),
}

/// Configuration for API services
#[derive(Debug)]
pub struct ApiConfig {
    pub fireworks_key: SecretBox<String>,
    pub groq_key: SecretBox<String>,
    pub elevenlabs_key: SecretBox<String>,
}

impl ApiConfig {
    /// Load API configuration from environment variables
    pub fn load() -> Result<Self, ConfigError> {
        // Load .env file if it exists (for development)
        dotenvy::dotenv().ok(); // Don't error if .env doesn't exist

        let fireworks_key = Self::load_api_key("FIREWORKS_API_KEY", "Fireworks AI")?;
        let groq_key = Self::load_api_key("GROQ_API_KEY", "Groq")?;
        let elevenlabs_key = Self::load_api_key("ELEVENLABS_API_KEY", "ElevenLabs")?;

        Ok(Self {
            fireworks_key,
            groq_key,
            elevenlabs_key,
        })
    }

    /// Load and validate a single API key from environment
    fn load_api_key(env_var: &str, service_name: &str) -> Result<SecretBox<String>, ConfigError> {
        let key = env::var(env_var).map_err(|_| ConfigError::MissingEnvVar(env_var.to_string()))?;

        // Basic validation - ensure key isn't empty
        if key.trim().is_empty() {
            return Err(ConfigError::InvalidKeyFormat {
                service: service_name.to_string(),
                reason: "API key cannot be empty".to_string(),
            });
        }

        // Optional: Add service-specific key format validation
        Self::validate_key_format(&key, service_name)?;

        Ok(SecretBox::new(Box::new(key)))
    }

    /// Validate API key format for each service
    fn validate_key_format(key: &str, service: &str) -> Result<(), ConfigError> {
        match service {
            "Fireworks AI" => {
                // Fireworks keys typically start with "fw_"
                if !key.starts_with("fw_") {
                    return Err(ConfigError::InvalidKeyFormat {
                        service: service.to_string(),
                        reason: "Fireworks AI keys should start with 'fw_'".to_string(),
                    });
                }
            }
            "Groq" => {
                // Groq keys typically start with "gsk_"
                if !key.starts_with("gsk_") {
                    return Err(ConfigError::InvalidKeyFormat {
                        service: service.to_string(),
                        reason: "Groq keys should start with 'gsk_'".to_string(),
                    });
                }
            }
            "ElevenLabs" => {
                // ElevenLabs keys are typically hex strings
                if key.len() < 10 {
                    return Err(ConfigError::InvalidKeyFormat {
                        service: service.to_string(),
                        reason: "ElevenLabs keys should be at least 10 characters".to_string(),
                    });
                }
            }
            _ => {} // No validation for unknown services
        }
        Ok(())
    }

    /// Get Fireworks API key (use only when making API calls)
    pub fn fireworks_key(&self) -> &str {
        self.fireworks_key.expose_secret()
    }

    /// Get Groq API key (use only when making API calls)
    pub fn groq_key(&self) -> &str {
        self.groq_key.expose_secret()
    }

    /// Get ElevenLabs API key (use only when making API calls)
    pub fn elevenlabs_key(&self) -> &str {
        self.elevenlabs_key.expose_secret()
    }

    /// Test API connectivity (optional - for startup validation)
    pub async fn validate_keys(&self) -> Result<(), ConfigError> {
        // TODO: Implement basic connectivity tests
        // - Simple Fireworks API health check
        // - Simple Groq API health check
        // - Simple ElevenLabs API health check
        log::info!("API key validation not yet implemented");
        Ok(())
    }
}

/// Load configuration with helpful error messages for development
pub fn load_config() -> Result<ApiConfig, ConfigError> {
    match ApiConfig::load() {
        Ok(config) => {
            log::info!("Successfully loaded API configuration");
            Ok(config)
        }
        Err(ConfigError::MissingEnvVar(var)) => {
            log::error!("Missing required environment variable: {}", var);
            log::error!("Create a .env file in the project root with:");
            log::error!("{}=your_api_key_here", var);
            Err(ConfigError::MissingEnvVar(var))
        }
        Err(e) => {
            log::error!("Configuration error: {}", e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_validation() {
        // Test Fireworks key validation
        assert!(ApiConfig::validate_key_format("fw_test123", "Fireworks AI").is_ok());
        assert!(ApiConfig::validate_key_format("invalid", "Fireworks AI").is_err());

        // Test Groq key validation
        assert!(ApiConfig::validate_key_format("gsk_test123", "Groq").is_ok());
        assert!(ApiConfig::validate_key_format("invalid", "Groq").is_err());

        // Test ElevenLabs key validation
        assert!(ApiConfig::validate_key_format("1234567890abcdef", "ElevenLabs").is_ok());
        assert!(ApiConfig::validate_key_format("short", "ElevenLabs").is_err());
    }
}
