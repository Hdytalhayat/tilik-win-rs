use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default RAM limit for the index writer (50 MiB).
pub const DEFAULT_WRITER_RAM_BYTES: u64 = 50 * 1024 * 1024;
/// Default global hotkey.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+F";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Folders to index.
    pub roots: Vec<String>,
    /// Excluded folder names and file extensions (case-insensitive).
    pub excluded: Vec<String>,
    /// Maximum RAM (bytes) for the index writer.
    pub writer_ram_bytes: u64,
    /// Global hotkey, e.g. "Ctrl+Alt+Space".
    pub hotkey: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            roots: Vec::new(),
            excluded: Vec::new(),
            writer_ram_bytes: DEFAULT_WRITER_RAM_BYTES,
            hotkey: DEFAULT_HOTKEY.to_string(),
        }
    }
}

impl Config {
    /// `%APPDATA%\localsearch-win\config.toml`
    pub fn default_path() -> PathBuf {
        let appdata = std::env::var("APPDATA")
            .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_default());
        Path::new(&appdata).join("localsearch-win").join("config.toml")
    }

    /// Loads the config at `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Saves the config to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// True if `path` is under an excluded folder name or has an excluded extension.
    pub fn is_excluded(&self, path: &Path) -> bool {
        let in_folder = path
            .components()
            .any(|c| {
                c.as_os_str()
                    .to_str()
                    .map(|name| self.excluded.iter().any(|e| name.eq_ignore_ascii_case(e)))
                    .unwrap_or(false)
            });
        if in_folder {
            return true;
        }
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                self.excluded
                    .iter()
                    .any(|x| x.trim_start_matches('.').eq_ignore_ascii_case(ext))
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_config_values() {
        let c = Config::default();
        assert!(c.roots.is_empty());
        assert!(c.excluded.is_empty());
        assert_eq!(c.writer_ram_bytes, DEFAULT_WRITER_RAM_BYTES);
        assert_eq!(c.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let c = Config::load(&dir.path().join("nope/config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("sub/config.toml");
        let mut c = Config::default();
        c.roots.push("C:\\docs".into());
        c.excluded.push("node_modules".into());
        c.excluded.push("tmp".into());
        c.writer_ram_bytes = 123;
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
    }

    #[test]
    fn partial_file_fills_missing_keys_with_defaults() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("config.toml");
        fs::write(&p, "hotkey = \"Ctrl+Shift+F\"\n").unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.hotkey, "Ctrl+Shift+F");
        assert_eq!(c.writer_ram_bytes, DEFAULT_WRITER_RAM_BYTES);
        assert!(c.roots.is_empty());
    }

    #[test]
    fn default_path_is_under_appdata() {
        let p = Config::default_path();
        let s = p.to_string_lossy().to_lowercase();
        assert!(s.ends_with("localsearch-win\\config.toml"), "{s}");
    }

    #[test]
    fn is_excluded_matches_folders_and_extensions() {
        let mut c = Config::default();
        c.excluded.push("node_modules".into());
        c.excluded.push("tmp".into());
        c.excluded.push("log".into());
        assert!(c.is_excluded(Path::new("C:/proj/node_modules/x.js")));
        assert!(c.is_excluded(Path::new("C:/proj/TMP/x.js")));
        assert!(c.is_excluded(Path::new("C:/proj/app.log")));
        assert!(c.is_excluded(Path::new("C:/proj/app.LOG")));
        assert!(!c.is_excluded(Path::new("C:/proj/src/main.rs")));
    }
}
