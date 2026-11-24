use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, OwnedValue, TantivyDocument};

use tantivy::{Index, IndexReader};

pub struct Searcher {
    parser: QueryParser,
    reader: IndexReader,
    path_field: Field,
    filename_field: Field,
    extension_field: Field,
    modified_ts_field: Field,
    content_field: Field,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub score: f32,
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub modified_ts: i64,
    pub snippet: String,
}

impl Searcher {
    pub fn new(index: &Index) -> anyhow::Result<Self> {
        let schema = index.schema().clone();
        let tokenizer = index.tokenizers().clone();
        let path_field = schema.get_field("path")?;
        let filename_field = schema.get_field("filename")?;
        let extension_field = schema.get_field("extension")?;
        let modified_ts_field = schema.get_field("modified_ts")?;
        let content_field = schema.get_field("content")?;
        let parser = QueryParser::new(
            schema.clone(),
            vec![filename_field, content_field, extension_field, path_field],
            tokenizer.clone(),
        );
        Ok(Self {
            parser,
            reader: index.reader()?,
            path_field,
            filename_field,
            extension_field,
            modified_ts_field,
            content_field,
        })
    }

    /// Searches `query`, returning up to `limit` hits with score, metadata,
    /// and a highlighted snippet (first 200 chars of content).
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Hit>> {
        let q = self.parser.parse_query(query)?;
        let searcher = self.reader.searcher();
        let top = searcher.search(&q, &TopDocs::with_limit(limit))?;
        let terms = query_terms(query);
        let mut hits = Vec::new();
        for (score, addr) in top {
            let d = searcher.doc::<TantivyDocument>(addr)?;
            let get = |f: Field| d.get_first(f).map(|v| v.into());
            let str_of = |v: Option<OwnedValue>| match v {
                Some(OwnedValue::Str(s)) => s,
                _ => String::new(),
            };
            let content = str_of(get(self.content_field));
            let snippet_src: String = content.chars().take(200).collect();
            hits.push(Hit {
                score,
                path: str_of(get(self.path_field)),
                filename: str_of(get(self.filename_field)),
                extension: str_of(get(self.extension_field)),
                modified_ts: match get(self.modified_ts_field) {
                    Some(OwnedValue::I64(n)) => n,
                    _ => 0,
                },
                snippet: highlight(&snippet_src, &terms),
            });
        }
        Ok(hits)
    }
}

/// Lowercased alphanumeric tokens of the query.
fn query_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Wraps case-insensitive occurrences of any term in `<b>…</b>`.
/// shortcut: no phrase/regex support; fine for keyword highlighting.
fn highlight(text: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let n = chars.len().min(lower.len());
    let mut out = String::with_capacity(text.len() + 32);
    let mut i = 0;
    while i < n {
        let mut matched = false;
        for term in terms {
            let t: Vec<char> = term.chars().collect();
            if i + t.len() <= n && &lower[i..i + t.len()] == t.as_slice() {
                out.push_str("<b>");
                out.extend(chars[i..i + t.len()].iter());
                out.push_str("</b>");
                i += t.len();
                matched = true;
                break;
            }
        }
        if !matched {
            out.push(chars[i]);
            i += 1;
        }
    }
    if n < chars.len() {
        out.extend(chars[n..].iter());
    }
    out
}
