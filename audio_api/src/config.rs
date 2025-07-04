use anyhow::Result;
use dotenvy::dotenv;
use std::path::PathBuf;

pub fn load_env() -> Result<()> {
    dotenv().ok();
    Ok(())
}

pub fn get_config_path() -> PathBuf {
    std::env::var("CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.json"))
}
