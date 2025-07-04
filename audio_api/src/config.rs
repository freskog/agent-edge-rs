use anyhow::Result;
use dotenvy::dotenv;

pub fn load_env() -> Result<()> {
    dotenv().ok();
    Ok(())
}
