use std::path::PathBuf;

use rayon::prelude::*;
use walkdir::WalkDir;

use crate::extractor::{extract_text, should_ignore};

pub struct Indexer {
    root: PathBuf,
}

impl Indexer {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Walks `root` in parallel, returning (path, text) for each text file.
    pub fn files(&self) -> Vec<(PathBuf, String)> {
        let paths: Vec<PathBuf> = WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && !should_ignore(e.path()))
            .map(|e| e.path().to_path_buf())
            .collect();

        paths.par_iter()
            .filter_map(|p| extract_text(p).ok().map(|t| (p.clone(), t)))
            .filter(|(_, t)| !t.is_empty())
            .collect()
    }
}
