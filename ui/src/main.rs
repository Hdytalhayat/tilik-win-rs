use std::path::PathBuf;

use tilik_core::indexer::Indexer;
use tilik_core::searcher::Searcher;

struct TilikApp {
    root: PathBuf,
    index_dir: PathBuf,
    query: String,
    results: Vec<String>,
    status: String,
}

impl TilikApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let root = PathBuf::from(".");
        Self {
            index_dir: root.join(".tilik-index"),
            root,
            query: String::new(),
            results: Vec::new(),
            status: String::new(),
        }
    }

    fn search(&mut self) {
        if self.query.trim().is_empty() {
            return;
        }
        let indexer = match Indexer::open_or_create(&self.index_dir) {
            Ok(x) => x,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        if let Err(e) = indexer.index_directory(&self.root) {
            self.status = e.to_string();
            return;
        }
        let searcher = match Searcher::new(indexer.index()) {
            Ok(s) => s,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        match searcher.search(&self.query, 20) {
            Ok(hits) => {
                self.results = hits
                    .iter()
                    .map(|h| format!("{:.2}  {} — {}", h.score, h.path, h.snippet))
                    .collect();
                self.status = String::new();
            }
            Err(e) => self.status = e.to_string(),
        }
    }
}

impl eframe::App for TilikApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.query);
                if ui.button("Search").clicked() {
                    self.search();
                }
            });
            if !self.status.is_empty() {
                ui.colored_label(eframe::egui::Color32::RED, &self.status);
            }
            for r in &self.results {
                ui.label(r);
            }
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Tilik",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(TilikApp::new(cc)))),
    )
}
