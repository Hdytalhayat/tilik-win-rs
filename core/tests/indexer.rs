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

#[test]
fn adding_second_folder_keeps_first_folder_results() {
    let dir = tempdir().unwrap();
    let root1 = dir.path().join("root1");
    let root2 = dir.path().join("root2");
    fs::create_dir_all(&root1).unwrap();
    fs::create_dir_all(&root2).unwrap();
    fs::write(root1.join("first.txt"), "alpha from first folder").unwrap();
    fs::write(root2.join("second.txt"), "beta from second folder").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    indexer.index_directory(&root1).unwrap();
    indexer.index_directory(&root2).unwrap();

    let searcher = Searcher::new(indexer.index()).unwrap();
    assert_eq!(searcher.search("alpha", 10).unwrap().len(), 1);
    assert_eq!(searcher.search("beta", 10).unwrap().len(), 1);
}

#[test]
fn new_text_file_is_detected_on_reindex() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("old.txt"), "old content").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    indexer.index_directory(&root).unwrap();

    fs::write(root.join("new.txt"), "freshly added text").unwrap();
    let report = indexer.index_directory(&root).unwrap();
    assert!(report.indexed >= 1);

    let searcher = Searcher::new(indexer.index()).unwrap();
    assert_eq!(searcher.search("freshly", 10).unwrap().len(), 1);
}

#[test]
fn removed_folder_is_cleaned_from_shared_index() {
    let dir = tempdir().unwrap();
    let root1 = dir.path().join("root1");
    let root2 = dir.path().join("root2");
    fs::create_dir_all(&root1).unwrap();
    fs::create_dir_all(&root2).unwrap();
    fs::write(root1.join("gone.txt"), "remove me").unwrap();
    fs::write(root2.join("stay.txt"), "keep me").unwrap();

    let indexer = Indexer::open_or_create(&dir.path().join("index")).unwrap();
    indexer.index_directory(&root1).unwrap();
    indexer.index_directory(&root2).unwrap();

    indexer
        .index_directories(&[root2.to_string_lossy().to_string()])
        .unwrap();

    let searcher = Searcher::new(indexer.index()).unwrap();
    assert!(searcher.search("remove", 10).unwrap().is_empty());
    assert_eq!(searcher.search("keep", 10).unwrap().len(), 1);
}
