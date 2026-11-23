use std::path::Path;
use std::sync::mpsc;

use notify::{recommended_watcher, RecursiveMode, Watcher};

/// Watches `path` recursively; sends each changed path string on `tx`.
/// The returned watcher must be kept alive for events to flow.
pub fn watch(
    path: &Path,
    tx: mpsc::Sender<String>,
) -> anyhow::Result<notify::RecommendedWatcher> {
    let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            for p in event.paths {
                let _ = tx.send(p.to_string_lossy().into_owned());
            }
        }
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(watcher)
}
