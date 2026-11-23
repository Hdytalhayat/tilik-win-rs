use std::fs;
use std::io::Read;
use std::path::Path;

use crate::error::ExtractError;

/// Maximum size of a file to extract (25 MiB).
pub const MAX_FILE_SIZE: u64 = 25 * 1024 * 1024;

const JUNK_DIRS: [&str; 5] =
    ["node_modules", ".git", "target", "$RECYCLE.BIN", ".tilik-index"];

/// Returns true if any component of `path` is a system/junk directory.
pub fn should_ignore(path: &Path) -> bool {
    path.components().any(|c| match c.as_os_str().to_str() {
        Some(name) => JUNK_DIRS.iter().any(|j| j.eq_ignore_ascii_case(name)),
        None => false,
    })
}

/// Returns true for common video/media extensions.
pub fn is_video_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "3gp"
    )
}

/// Returns true when a snippet is safe/readable enough to show as text preview.
pub fn is_text_preview_available(ext: &str, snippet: &str) -> bool {
    if snippet.trim().is_empty() {
        return false;
    }
    if snippet.contains('\u{FFFD}') {
        return false;
    }
    if snippet.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
        return false;
    }
    let ext = ext.to_ascii_lowercase();
    if is_video_extension(&ext) {
        return false;
    }
    !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "mp3" | "wav" | "flac" | "ogg" | "zip" | "exe" | "dll"
    )
}

/// Extracts readable text from `path`.
/// Never panics: locked, missing, oversized, or corrupted files yield `Err`.
/// Text formats are decoded lossily, so non-UTF-8 content never fails.
pub fn extract_text(path: &Path) -> Result<String, ExtractError> {
    let len = fs::metadata(path)?.len();
    if len > MAX_FILE_SIZE {
        return Err(ExtractError::TooLarge(len, MAX_FILE_SIZE));
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => extract_pdf(path),
        "docx" => extract_zip_entry(path, "word/document.xml"),
        "xlsx" => extract_zip_entry(path, "xl/sharedStrings.xml"),
        // Images have no extractable text; index metadata only.
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" => Ok(String::new()),
        // txt, md, rs, py, js, ts, json, csv, and anything else: raw text
        _ => Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned()),
    }
}

/// PDF: decode each page's content stream and pull text out of `Tj`/`TJ` operators.
/// shortcut: ignores hex strings and ToUnicode CMap mapping; fine for simple PDFs.
fn extract_pdf(path: &Path) -> Result<String, ExtractError> {
    let doc =
        lopdf::Document::load(path).map_err(|e| ExtractError::Corrupted(e.to_string()))?;
    let mut text = String::new();
    for page_id in doc.get_pages().values() {
        if let Ok(content) = doc.get_page_content(*page_id) {
            let t = pdf_text(&String::from_utf8_lossy(&content));
            if !t.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t);
            }
        }
    }
    if text.is_empty() {
        return Err(ExtractError::Corrupted("no extractable text in pdf".into()));
    }
    Ok(text)
}

/// Collects text from `(literal) Tj` and `[(lit) num (lit)] TJ` in raw content.
fn pdf_text(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                let (s, next) = read_literal(&chars, i);
                let after = skip_ws(&chars, next);
                if after < chars.len() && chars[after..].starts_with(&['T', 'j']) {
                    out.push_str(&s);
                    out.push(' ');
                }
                i = next;
            }
            '[' => {
                let mut depth = 1;
                let mut j = i + 1;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let array: String = chars[i + 1..j - 1].iter().collect();
                let after = skip_ws(&chars, j);
                if after < chars.len() && chars[after..].starts_with(&['T', 'J']) {
                    for s in literals_in(&array) {
                        out.push_str(&s);
                        out.push(' ');
                    }
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    out.trim().to_string()
}

/// Reads a PDF string literal starting at `(`; returns (text, index after `)`).
fn read_literal(chars: &[char], start: usize) -> (String, usize) {
    let mut s = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 1;
                if i < chars.len() {
                    match chars[i] {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        c => s.push(c),
                    }
                }
            }
            ')' => return (s, i + 1),
            c => s.push(c),
        }
        i += 1;
    }
    (s, i)
}

/// All string literals inside a `TJ` array body.
fn literals_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let (s, next) = read_literal(&chars, i);
            out.push(s);
            i = next;
        } else {
            i += 1;
        }
    }
    out
}

fn skip_ws(chars: &[char], i: usize) -> usize {
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn build_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, content) in entries {
            zip.start_file(name, zip::write::FileOptions::<()>::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn build_minimal_pdf(path: &Path, text: &str) {
        let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let mut buf: Vec<u8> = Vec::new();
        let mut offsets: Vec<usize> = vec![0];
        buf.extend_from_slice(b"%PDF-1.4\n");
        let mut obj = |n: u32, body: &str| {
            offsets.push(buf.len());
            buf.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
        };
        obj(1, "<</Type /Catalog /Pages 2 0 R>>");
        obj(2, "<</Type /Pages /Kids [3 0 R] /Count 1>>");
        obj(3, "<</Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources <</Font <</F1 5 0 R>>>>>>");
        obj(4, &format!("<</Length {}>>\nstream\n{content}\nendstream", content.len()));
        obj(5, "<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>");
        let xref_pos = buf.len();
        let mut xref = String::from("xref\n0 6\n0000000000 65535 f \n");
        for off in &offsets[1..] {
            xref.push_str(&format!("{off:010} 00000 n \n"));
        }
        buf.extend_from_slice(xref.as_bytes());
        buf.extend_from_slice(
            format!("trailer\n<</Size 6 /Root 1 0 R>>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes(),
        );
        fs::write(path, buf).unwrap();
    }

    #[test]
    fn image_files_have_no_text_preview() {
        let dir = tempdir().unwrap();
        for name in ["a.png", "b.jpg", "c.jpeg", "d.webp", "e.gif", "f.bmp", "g.ico"] {
            let p = dir.path().join(name);
            fs::write(&p, [0u8, 1, 2, 3]).unwrap();
            assert_eq!(extract_text(&p).unwrap(), "");
        }
    }

    #[test]
    fn extracts_text_formats() {
        let dir = tempdir().unwrap();
        let cases = [
            ("a.txt", "plain text"),
            ("b.md", "# Heading\nbody"),
            ("c.json", "{\"key\": \"value\"}"),
            ("d.csv", "a,b\n1,2"),
            ("e.rs", "fn main() {}"),
            ("f.py", "print('hi')"),
            ("g.js", "const x = 1;"),
            ("h.ts", "const x: number = 1;"),
        ];
        for (name, content) in cases {
            let p = dir.path().join(name);
            fs::write(&p, content).unwrap();
            assert_eq!(extract_text(&p).unwrap(), content);
        }
    }

    #[test]
    fn extracts_docx_text() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("doc.docx");
        build_zip(
            &p,
            &[(
                "word/document.xml",
                "<w:document><w:body><w:p><w:t>Hello Docx</w:t></w:p></w:body></w:document>",
            )],
        );
        assert!(extract_text(&p).unwrap().contains("Hello Docx"));
    }

    #[test]
    fn extracts_xlsx_strings() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("data.xlsx");
        build_zip(
            &p,
            &[("xl/sharedStrings.xml", "<sst><si><t>Hello Xlsx</t></si></sst>")],
        );
        assert!(extract_text(&p).unwrap().contains("Hello Xlsx"));
    }

    #[test]
    fn extracts_pdf_text() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("doc.pdf");
        build_minimal_pdf(&p, "Hello PDF");
        assert!(extract_text(&p).unwrap().contains("Hello PDF"));
    }

    #[test]
    fn corrupted_files_error_instead_of_panicking() {
        let dir = tempdir().unwrap();
        for name in ["bad.pdf", "bad.docx", "bad.xlsx"] {
            let p = dir.path().join(name);
            fs::write(&p, b"this is not a valid file").unwrap();
            assert!(extract_text(&p).is_err(), "{name} should fail");
        }
    }

    #[test]
    fn empty_non_utf8_and_missing_files_do_not_panic() {
        let dir = tempdir().unwrap();
        let empty = dir.path().join("empty.txt");
        fs::write(&empty, b"").unwrap();
        assert_eq!(extract_text(&empty).unwrap(), "");

        let weird = dir.path().join("weird.txt");
        fs::write(&weird, b"\xFF\xFEgarbage").unwrap();
        assert!(extract_text(&weird).unwrap().contains("garbage"));

        assert!(extract_text(&dir.path().join("missing.txt")).is_err());
    }

    #[test]
    fn rejects_oversized_files() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("big.txt");
        fs::write(&p, vec![b'x'; MAX_FILE_SIZE as usize + 1]).unwrap();
        assert!(matches!(
            extract_text(&p),
            Err(ExtractError::TooLarge(len, MAX_FILE_SIZE)) if len > MAX_FILE_SIZE
        ));
    }

    #[test]
    fn ignores_junk_directories() {
        for name in ["node_modules", ".git", "target", "$RECYCLE.BIN"] {
            assert!(should_ignore(Path::new(&format!("C:/proj/{name}/file.txt"))));
        }
        assert!(!should_ignore(Path::new("C:/proj/src/main.rs")));
    }
}

/// docx/xlsx: read one XML entry from the OPC zip and strip the tags.
/// shortcut: xlsx only reads sharedStrings (numeric/inline cells missed); XML entities stay unescaped.
fn extract_zip_entry(path: &Path, entry: &str) -> Result<String, ExtractError> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)
        .map_err(|e| ExtractError::Corrupted(e.to_string()))?;
    let mut file = zip
        .by_name(entry)
        .map_err(|e| ExtractError::Corrupted(e.to_string()))?;
    let mut xml = String::new();
    file.read_to_string(&mut xml)
        .map_err(|e| ExtractError::Corrupted(e.to_string()))?;
    Ok(strip_xml_tags(&xml))
}

/// Removes XML tags, keeping text content.
fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}
