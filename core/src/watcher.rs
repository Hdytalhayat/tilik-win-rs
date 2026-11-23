use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{recommended_watcher, RecursiveMode, Watcher as _};

/// Default debounce window: events are coalesced until the filesystem is
/// quiet for this long (e.g. a Word/VS Code save fires many rapid events).
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// Live file watcher. Changed paths accumulate until the filesystem is
/// quiet for `debounce`, then one deduplicated batch is sent on `tx`.
/// Must be kept alive; dropping it stops the watcher and the debounce thread.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
    _thread: thread::JoinHandle<()>,
}

pub fn watch(paths: &[impl AsRef<Path>], tx: mpsc::Sender<Vec<String>>) -> anyhow::Result<Watcher> {
    watch_with_debounce(paths, tx, DEBOUNCE)
}

pub fn watch_with_debounce(
    paths: &[impl AsRef<Path>],
    tx: mpsc::Sender<Vec<String>>,
    debounce: Duration,
) -> anyhow::Result<Watcher> {
    let (event_tx, event_rx) = mpsc::channel::<String>();
    let mut inner = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            for p in event.paths {
                let _ = event_tx.send(p.to_string_lossy().into_owned());
            }
        }
    })?;
    for p in paths {
        inner.watch(p.as_ref(), RecursiveMode::Recursive)?;
    }
    let _thread = thread::spawn(move || drain_debounced(event_rx, tx, debounce));
    Ok(Watcher {
        _inner: inner,
        _thread,
    })
}

/// Collects paths until `debounce` passes with no new event, then sends the
/// deduplicated batch. Exits when the event channel is disconnected.
fn drain_debounced(
    rx: mpsc::Receiver<String>,
    tx: mpsc::Sender<Vec<String>>,
    debounce: Duration,
) {
    loop {
        let mut batch: HashSet<String> = match rx.recv() {
            Ok(first) => {
                let mut b = HashSet::new();
                b.insert(first);
                b
            }
            Err(_) => return, // watcher dropped
        };
        loop {
            match rx.recv_timeout(debounce) {
                Ok(p) => {
                    batch.insert(p);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if tx.send(batch.into_iter().collect()).is_err() {
            return; // consumer gone
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const FAST: Duration = Duration::from_millis(50);

    #[test]
    fn rapid_events_coalesce_into_one_batch() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch_with_debounce(&[dir.path()], tx, FAST).unwrap();
        for i in 0..3 {
            fs::write(dir.path().join(format!("f{i}.txt")), format!("{i}")).unwrap();
        }
        let batch = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for i in 0..3 {
            assert!(
                batch.iter().any(|p| p.ends_with(format!("f{i}.txt").as_str())),
                "missing f{i}.txt in batch: {batch:?}"
            );
        }
    }

    #[test]
    fn repeated_changes_dedupe_to_one_path() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch_with_debounce(&[dir.path()], tx, FAST).unwrap();
        let p = dir.path().join("a.txt");
        for i in 0..3 {
            fs::write(&p, format!("{i}")).unwrap();
        }
        let batch = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let hits: Vec<&String> = batch.iter().filter(|s| s.ends_with("a.txt")).collect();
        assert_eq!(hits.len(), 1, "a.txt should appear once: {batch:?}");
    }

    #[test]
    fn separate_batches_after_quiet_period() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch_with_debounce(&[dir.path()], tx, FAST).unwrap();
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        let b1 = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(b1.iter().any(|p| p.ends_with("a.txt")), "{b1:?}");

        thread::sleep(FAST * 3); // longer than the debounce window
        fs::write(dir.path().join("b.txt"), "two").unwrap();
        let b2 = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(b2.iter().any(|p| p.ends_with("b.txt")), "{b2:?}");
        assert!(!b2.iter().any(|p| p.ends_with("a.txt")), "{b2:?}");
    }
}
