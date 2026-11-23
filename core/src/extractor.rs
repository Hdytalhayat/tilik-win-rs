use std::fs;
use std::path::Path;

use crate::error::ExtractError;

/// Reads a file's text content. Binary files (NUL in first 8 KiB) yield an error.
/// shortcut: NUL sniffing only; replace with encoding sniffing if non-UTF-8 sources appear.
pub fn extract_text(path: &Path) -> Result<String, ExtractError> {
    let bytes = fs::read(path)?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Err(ExtractError::Binary);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_rejects_binary() {
        let dir = std::env::temp_dir().join("tilik-test");
        fs::create_dir_all(&dir).unwrap();
        let text_file = dir.join("a.txt");
        fs::write(&text_file, "hello").unwrap();
        assert_eq!(extract_text(&text_file).unwrap(), "hello");

        let bin_file = dir.join("b.bin");
        fs::write(&bin_file, [0u8, 1, 2]).unwrap();
        assert!(matches!(extract_text(&bin_file), Err(ExtractError::Binary)));
    }
}
