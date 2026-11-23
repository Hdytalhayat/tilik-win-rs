use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tantivy::directory::MmapDirectory;
use tantivy::schema::{
    Field, Schema, SchemaBuilder, TantivyDocument, INDEXED, STORED, STRING, TEXT,
};
use tantivy::{Index, Term};
use walkdir::WalkDir;

use crate::extractor::{extract_text, should_ignore};

/// Index schema: single source of truth shared by indexer and searcher.
pub fn build_schema() -> Schema {
    let mut b = SchemaBuilder::new();
    b.add_text_field("path", STORED | STRING);
    b.add_text_field("filename", STORED | TEXT);
    b.add_text_field("extension", STORED | STRING);
    b.add_i64_field("modified_ts", INDEXED | STORED);
    b.add_text_field("content", STORED | TEXT);
    b.build()
}

/// (mtime, size) of each indexed path. Persisted so incremental skip/delete
/// decisions survive across `Indexer` instances (the UI reopens the index).
#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    files: BTreeMap<String, [i64; 2]>,
}

#[derive(Debug)]
pub struct Report {
    pub indexed: usize,
    pub skipped: usize,
    pub deleted: usize,
}

/// Monitored progress of the current `index_directory` run.
#[derive(Clone)]
pub struct Progress {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
}

impl Progress {
    pub fn ratio(&self) -> f64 {
        let done = self.done.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            1.0
        } else {
            done as f64 / total as f64
        }
    }
}

struct FileDoc {
    path: String,
    filename: String,
    extension: String,
    mtime: i64,
    size: u64,
    text: String,
}

struct Fields {
    path: Field,
    filename: Field,
    extension: Field,
    modified_ts: Field,
    content: Field,
}

pub struct Indexer {
    index: Index,
    fields: Fields,
    manifest_path: std::path::PathBuf,
    manifest: RwLock<BTreeMap<String, (i64, u64)>>,
    progress: Progress,
}

impl Indexer {
    pub fn open_or_create(index_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(index_dir)?;
        let schema = build_schema();
        let dir = MmapDirectory::open(index_dir)?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let fields = Fields {
            path: schema.get_field("path")?,
            filename: schema.get_field("filename")?,
            extension: schema.get_field("extension")?,
            modified_ts: schema.get_field("modified_ts")?,
            content: schema.get_field("content")?,
        };
        let manifest_path = index_dir.join("manifest.toml");
        let manifest: BTreeMap<String, (i64, u64)> = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| toml::from_str::<Manifest>(&s).ok())
            .map(|m| {
                m.files
                    .into_iter()
                    .map(|(k, [mt, sz])| (k, (mt, sz as u64)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            index,
            fields,
            manifest_path,
            manifest: RwLock::new(manifest),
            progress: Progress {
                done: Arc::new(AtomicUsize::new(0)),
                total: Arc::new(AtomicUsize::new(0)),
            },
        })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }

    /// Indexes one folder while preserving documents from other indexed folders.
    /// Only files that vanished from `path` are removed.
    pub fn index_directory(&self, path: &Path) -> anyhow::Result<Report> {
        self.index_roots(&[path.to_path_buf()], false)
    }

    /// Indexes all given folders in one shared index and removes documents
    /// that belong to folders no longer present in `roots`.
    pub fn index_directories(&self, roots: &[String]) -> anyhow::Result<Report> {
        self.index_roots(
            &roots.iter().map(PathBuf::from).collect::<Vec<_>>(),
            true,
        )
    }

    fn index_roots(&self, roots: &[PathBuf], prune_other_roots: bool) -> anyhow::Result<Report> {
        // Limit CPU usage: at most half the logical cores, at least 2 threads.
        let logical = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let threads = (logical / 2).max(2);
        let _ = rayon::ThreadPoolBuilder::new().num_threads(threads).build_global();

        // 1. Walk all requested roots.
        let mut entries: Vec<(PathBuf, i64, u64)> = Vec::new();
        for root in roots {
            if !root.exists() {
                continue;
            }
            for e in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
                if e.file_type().is_file() && !should_ignore(e.path()) {
                    let md = e.metadata()?;
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    entries.push((e.path().to_path_buf(), mtime, md.len()));
                }
            }
        }

        // 2. Parallel extraction with progress.
        self.progress.total.store(entries.len(), Ordering::Relaxed);
        self.progress.done.store(0, Ordering::Relaxed);
        let docs: Vec<Option<FileDoc>> = entries
            .par_iter()
            .map(|(p, mtime, size)| {
                let doc = extract_text(p)
                    .ok()
                    .map(|text| FileDoc {
                        path: p.to_string_lossy().into_owned(),
                        filename: p
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        extension: p
                            .extension()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        mtime: *mtime,
                        size: *size,
                        text,
                    });
                self.progress.done.fetch_add(1, Ordering::Relaxed);
                doc
            })
            .collect();

        // 3. Skip / update / delete pass.
        let old = self.manifest.read().unwrap().clone();
        let current_paths: HashSet<String> =
            docs.iter().flatten().map(|d| d.path.clone()).collect();
        let mut new_manifest = if prune_other_roots {
            BTreeMap::new()
        } else {
            old.iter()
                .filter(|(p, _)| !roots.iter().any(|r| Path::new(p).starts_with(r)))
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        };
        let mut report = Report {
            indexed: 0,
            skipped: 0,
            deleted: 0,
        };
        let heap = threads * 16 * 1024 * 1024;
        let mut writer = self.index.writer_with_num_threads(threads, heap)?;
        for doc in docs.iter().flatten() {
            match old.get(&doc.path) {
                Some(&(mt, sz)) if mt == doc.mtime && sz == doc.size => {
                    report.skipped += 1;
                    new_manifest.insert(doc.path.clone(), (doc.mtime, doc.size));
                }
                _ => {
                    writer.delete_term(Term::from_field_text(self.fields.path, &doc.path));
                    let mut d = TantivyDocument::new();
                    d.add_text(self.fields.path, &doc.path);
                    d.add_text(self.fields.filename, &doc.filename);
                    d.add_text(self.fields.extension, &doc.extension);
                    d.add_i64(self.fields.modified_ts, doc.mtime);
                    d.add_text(self.fields.content, &doc.text);
                    writer.add_document(d)?;
                    report.indexed += 1;
                    new_manifest.insert(doc.path.clone(), (doc.mtime, doc.size));
                }
            }
        }
        for path in old.keys() {
            let under_target = roots.iter().any(|r| Path::new(path).starts_with(r));
            let should_delete = if prune_other_roots {
                !current_paths.contains(path)
            } else {
                under_target && !current_paths.contains(path)
            };
            if should_delete {
                writer.delete_term(Term::from_field_text(self.fields.path, path));
                report.deleted += 1;
                new_manifest.remove(path);
            }
        }
        writer.commit()?;

        // 4. Persist the manifest.
        let m = Manifest {
            files: new_manifest
                .iter()
                .map(|(k, &(mt, sz))| (k.clone(), [mt, sz as i64]))
                .collect(),
        };
        fs::write(&self.manifest_path, toml::to_string(&m)?)?;
        *self.manifest.write().unwrap() = new_manifest;
        Ok(report)
    }
}