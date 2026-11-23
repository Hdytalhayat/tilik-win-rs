use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub root: String,
    pub index_dir: String,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
