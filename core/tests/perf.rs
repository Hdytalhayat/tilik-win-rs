use std::fs;
use std::path::Path;
use std::time::Instant;

use tempfile::tempdir;
use tilik_core::indexer::Indexer;
use tilik_core::searcher::Searcher;

const FILE_COUNT: usize = 5000;

fn gen_files(root: &Path) {
    let exts = ["txt", "md", "json", "csv"];
    for i in 0..FILE_COUNT {
        let ext = exts[i % exts.len()];
        let sub = root.join(format!("bucket{}", i % 10));
        fs::create_dir_all(&sub).unwrap();
        let body = format!(
            "Document {i}. keyword needle performance search data. \
             Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
             File number {i} in bucket {}. searchable token alpha beta gamma.",
            i % 10
        );
        fs::write(sub.join(format!("file_{i}.{ext}")), body).unwrap();
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(m) = p.metadata() {
                total += m.len();
            }
        }
    }
    total
}

#[test]
#[ignore = "heavy performance benchmark; run with --ignored --nocapture"]
fn perf_index_and_search() {
    let td = tempdir().unwrap();
    let data = td.path().join("data");
    let index = td.path().join("index");
    fs::create_dir_all(&data).unwrap();

    let t_gen = Instant::now();
    gen_files(&data);
    let gen_ms = t_gen.elapsed().as_millis();

    let indexer = Indexer::open_or_create(&index).unwrap();
    let t_idx = Instant::now();
    let report = indexer.index_directory(&data).unwrap();
    let idx_ms = t_idx.elapsed().as_millis();

    let idx_bytes = dir_size(&index);

    let searcher = Searcher::new(indexer.index()).unwrap();
    // warm-up
    let _ = searcher.search("needle", 50);

    let queries = ["needle", "performance", "search", "document", "missingtoken"];
    let mut latencies = Vec::new();
    for q in queries {
        let t = Instant::now();
        for _ in 0..50 {
            let _ = searcher.search(q, 50).unwrap();
        }
        let per = t.elapsed().as_micros() / 50;
        latencies.push((q, per));
    }

    println!("PERF files={FILE_COUNT} gen_ms={gen_ms} indexed={} skipped={} deleted={} idx_ms={idx_ms} idx_bytes={idx_bytes}", report.indexed, report.skipped, report.deleted);
    for (q, us) in &latencies {
        println!("PERF query={q} avg_us={us}");
    }
}