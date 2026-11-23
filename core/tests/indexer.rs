use std::fs;
use std::time::Duration;
use tempfile::tempdir;

use tilik_core::indexer::Indexer;
use tilik_core::searcher::Searcher;

#[test]
fn indexes_and_searches() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "the quick brown fox jumps").unwrap();
    fs::write(root.join("b.md"), "# dogs\nlazy dogs sleep").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    let report = indexer.index_directory(&root).unwrap();
    assert_eq!(report.indexed, 2);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.deleted, 0);
    assert_eq!(indexer.progress().ratio(), 1.0);

    let searcher = Searcher::new(indexer.index()).unwrap();
    let hits = searcher.search("quick", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].path.ends_with("a.txt"));
    assert!(hits[0].score > 0.0);
    assert_eq!(hits[0].filename, "a.txt");
    assert_eq!(hits[0].extension, "txt");
    assert!(hits[0].snippet.contains("<b>quick</b>"), "snippet: {}", hits[0].snippet);
}

#[test]
fn incremental_update_skips_unchanged_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    fs::write(&a, "alpha one").unwrap();
    fs::write(&b, "beta two").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    indexer.index_directory(&root).unwrap();

    // let the mtime tick past FS granularity, then change only a.txt
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(&a, "alpha one gamma").unwrap();

    let report = indexer.index_directory(&root).unwrap();
    assert_eq!(report.indexed, 1, "only a.txt should be re-indexed");
    assert_eq!(report.skipped, 1, "b.txt should be skipped");

    let searcher = Searcher::new(indexer.index()).unwrap();
    assert_eq!(searcher.search("gamma", 10).unwrap().len(), 1);
    assert_eq!(searcher.search("beta", 10).unwrap().len(), 1);
}

#[test]
fn deleted_files_are_removed_from_index() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    fs::write(&a, "keep me").unwrap();
    fs::write(&b, "delete me").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    indexer.index_directory(&root).unwrap();

    fs::remove_file(&b).unwrap();
    let report = indexer.index_directory(&root).unwrap();
    assert_eq!(report.deleted, 1);
    assert_eq!(report.indexed, 0);

    let searcher = Searcher::new(indexer.index()).unwrap();
    assert!(searcher.search("delete", 10).unwrap().is_empty());
    assert_eq!(searcher.search("keep", 10).unwrap().len(), 1);
}
