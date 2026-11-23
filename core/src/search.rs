use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, OwnedValue, Schema, SchemaBuilder, TantivyDocument, STORED, TEXT};
use tantivy::tokenizer::TokenizerManager;
use tantivy::{doc, Index, IndexReader};

use crate::indexer::Indexer;

pub struct SearchEngine {
    schema: Schema,
    tokenizer: TokenizerManager,
    reader: IndexReader,
    path_field: Field,
    content_field: Field,
}

impl SearchEngine {
    /// Builds a fresh tantivy index in `dir` from all text files under `root`.
    /// shortcut: full rebuild, no incremental updates; use open_or_create + merge for large trees.
    pub fn build(root: &Path, dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;

        let mut builder = SchemaBuilder::new();
        let path_field = builder.add_text_field("path", STORED | TEXT);
        let content_field = builder.add_text_field("content", STORED | TEXT);
        let schema = builder.build();

        let index = Index::create_in_dir(dir, schema.clone())?;
        let mut writer = index.writer(50_000)?;
        for (path, text) in Indexer::new(root).files() {
            writer.add_document(doc!(
                path_field => &*path.to_string_lossy(),
                content_field => text,
            ))?;
        }
        writer.commit()?;

        let reader = index.reader()?;
        Ok(Self {
            schema,
            tokenizer: index.tokenizers().clone(),
            reader,
            path_field,
            content_field,
        })
    }

    /// Full-text search; returns matching file paths, most relevant first (max 20).
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<String>> {
        let parser = QueryParser::new(
            self.schema.clone(),
            vec![self.path_field, self.content_field],
            self.tokenizer.clone(),
        );
        let q = parser.parse_query(query)?;
        let searcher = self.reader.searcher();
        let top = searcher.search(&q, &TopDocs::with_limit(20))?;
        let mut hits = Vec::new();
        for (_score, addr) in top {
            let stored = searcher.doc::<TantivyDocument>(addr)?;
            if let Some(OwnedValue::Str(path)) =
                stored.get_first(self.path_field).map(|v| v.into())
            {
                hits.push(path);
            }
        }
        Ok(hits)
    }
}
