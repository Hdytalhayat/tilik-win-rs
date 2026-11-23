use std::path::PathBuf;

use tilik_core::search::SearchEngine;

struct TilikApp {
    root: PathBuf,
    index_dir: PathBuf,
    query: String,
    results: Vec<String>,
    status: String,
    engine: Option<SearchEngine>,
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
            engine: None,
        }
    }

    fn search(&mut self) {
        if self.query.trim().is_empty() {
            return;
        }
        if self.engine.is_none() {
            match SearchEngine::build(&self.root, &self.index_dir) {
                Ok(engine) => self.engine = Some(engine),
                Err(e) => {
                    self.status = e.to_string();
                    return;
                }
            }
        }
        if let Some(engine) = &self.engine {
            match engine.search(&self.query) {
                Ok(results) => {
                    self.results = results;
                    self.status = String::new();
                }
                Err(e) => self.status = e.to_string(),
            }
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
