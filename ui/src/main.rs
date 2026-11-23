#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use tilik_core::config::Config;
use tilik_core::indexer::Indexer;
use tilik_core::searcher::{Hit, Searcher};
use tilik_core::watcher::{watch, Watcher};

mod platform;

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(80);
const SEARCH_LIMIT: usize = 50;

pub enum Cmd {
    Search(String, mpsc::Sender<Vec<Hit>>),
    SetRoots(Vec<String>),
    Reindex,
    ReindexRoots(Vec<String>),
}

enum Msg {
    Progress(f64),
    Done,
    Error(String),
}

struct App {
    query: String,
    prev_query: String,
    last_edit: Instant,
    last_searched: String,
    results: Vec<Hit>,
    selected: usize,
    cmd_tx: mpsc::Sender<Cmd>,
    msg_rx: mpsc::Receiver<Msg>,
    reply_rx: Option<mpsc::Receiver<Vec<Hit>>>,
    pending_query: String,
    progress: f64,
    indexing: bool,
    status: String,
    roots: Vec<String>,
    new_root: String,
    focused: bool,
    _platform: Option<platform::windows::Platform>,
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config = Config::load(&Config::default_path()).unwrap_or_default();
        let roots = config.roots.clone();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (msg_tx, msg_rx) = mpsc::channel();
        let index_dir = Config::default_path()
            .parent()
            .map(|p| p.join("index"))
            .unwrap_or_default();
        let worker_tx = cmd_tx.clone();
        std::thread::spawn(move || worker(worker_tx, cmd_rx, msg_tx, index_dir));
        let _ = cmd_tx.send(Cmd::SetRoots(roots.clone()));
        let platform = match platform::windows::init(&config.hotkey, cmd_tx.clone()) {
            Ok(p) => Some(p),
            Err(e) if e.to_string() == "already running" => {
                eprintln!("Tilik is already running.");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("platform init failed: {e}");
                None
            }
        };
        Self {
            query: String::new(),
            prev_query: String::new(),
            last_edit: Instant::now(),
            last_searched: String::new(),
            results: Vec::new(),
            selected: 0,
            cmd_tx,
            msg_rx,
            reply_rx: None,
            pending_query: String::new(),
            progress: 0.0,
            indexing: true,
            status: String::new(),
            roots,
            new_root: String::new(),
            focused: false,
            _platform: platform,
        }
    }

    fn open_selected(&self, in_explorer: bool) {
        let Some(h) = self.results.get(self.selected) else {
            return;
        };
        if in_explorer {
            let path = h.path.replace('/', "\\");
            let _ = std::process::Command::new("explorer")
                .raw_arg(format!("/select,\"{path}\""))
                .spawn();
        } else {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &h.path])
                .spawn();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, mut ctx: &egui::Context, _frame: &mut eframe::Frame) {
        platform::windows::publish_context(&ctx);

        // X button: hide to the tray, keep the background indexer alive.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }

        // 1. Drain worker messages (non-blocking).
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                Msg::Progress(r) => {
                    self.progress = r;
                    self.indexing = true;
                }
                Msg::Done => {
                    self.indexing = false;
                    self.progress = 1.0;
                }
                Msg::Error(e) => self.status = e,
            }
            ctx.request_repaint();
        }

        // 2. Collect finished search replies (reject stale ones).
        if let Some(rx) = self.reply_rx.as_ref() {
            if let Ok(hits) = rx.try_recv() {
                if self.pending_query == self.query.trim() {
                    self.results = hits;
                    self.selected = 0;
                    self.reply_rx = None;
                }
            }
        }
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }

        // 3. Debounced real-time search.
        if self.query != self.prev_query {
            self.prev_query = self.query.clone();
            self.last_edit = Instant::now();
        }
        let q = self.query.trim();
        if q != self.last_searched.as_str() {
            if q.is_empty() {
                self.last_searched.clear();
                self.results.clear();
                self.reply_rx = None;
            } else if Instant::now() - self.last_edit >= SEARCH_DEBOUNCE {
                self.last_searched = q.to_string();
                let (rtx, rrx) = mpsc::channel();
                self.pending_query = q.to_string();
                self.reply_rx = Some(rrx);
                let _ = self.cmd_tx.send(Cmd::Search(q.to_string(), rtx));
            }
        }

        // 4. Keyboard navigation.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }
        if !self.results.is_empty() {
            let (down, up, enter, ctrl) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::Enter),
                    i.modifiers.ctrl,
                )
            });
            if down {
                self.selected = (self.selected + 1).min(self.results.len() - 1);
            }
            if up {
                self.selected = self.selected.saturating_sub(1);
            }
            if enter {
                self.open_selected(ctrl);
            }
        }

        // 5. UI.
        egui::TopBottomPanel::top("tilik_top").show(&mut ctx, |ui| {
            let resp = ui.text_edit_singleline(&mut self.query);
            if !self.focused {
                resp.request_focus();
                self.focused = true;
            }
        });

        egui::SidePanel::right("tilik_preview")
            .min_width(240.0)
            .default_width(300.0)
            .show(&mut ctx, |ui| {
                ui.heading("Preview");
                ui.separator();
                match self.results.get(self.selected) {
                    Some(h) => {
                        let image_ext = matches!(
                            h.extension.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico"
                        );
                        if image_ext || h.snippet.trim().is_empty() {
                            ui.label("Text preview is not available for image/binary files.");
                        } else {
                            ui.label(h.snippet.replace("<b>", "").replace("</b>", ""));
                        }
                        ui.separator();
                        if let Ok(md) = std::fs::metadata(&h.path) {
                            ui.label(format!("Size: {}", fmt_bytes(md.len())));
                            ui.label(format!("Modified: {}", format_ts(h.modified_ts)));
                        }
                        ui.label(format!("Path: {}", h.path));
                    }
                    None => {
                        ui.label(if self.query.trim().is_empty() {
                            "Type to search."
                        } else {
                            "No results."
                        });
                    }
                }

                ui.separator();
                ui.heading("Indexed folders");
                let mut remove: Vec<usize> = Vec::new();
                for (i, r) in self.roots.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(r.clone());
                        if ui.small_button("×").clicked() {
                            remove.push(i);
                        }
                    });
                }
                for i in remove.into_iter().rev() {
                    self.roots.remove(i);
                }
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_root);
                    if ui.button("Add").clicked() {
                        let t = self.new_root.trim().replace('/', "\\");
                        if !t.is_empty() && !self.roots.contains(&t) {
                            self.roots.push(t);
                        }
                        self.new_root.clear();
                    }
                    if ui.button("Save").clicked() {
                        for r in &mut self.roots {
                            *r = r.trim().replace('/', "\\");
                        }
                        let mut cfg = Config::load(&Config::default_path()).unwrap_or_default();
                        cfg.roots = self.roots.clone();
                        if let Err(e) = cfg.save(&Config::default_path()) {
                            self.status = e.to_string();
                        } else {
                            let _ = self.cmd_tx.send(Cmd::SetRoots(self.roots.clone()));
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(&mut ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, h) in self.results.iter().enumerate() {
                    let r = ui.selectable_label(
                        i == self.selected,
                        format!("{}  ·  {}", h.filename, h.extension),
                    );
                    if r.clicked() {
                        self.selected = i;
                    }
                }
                if self.results.is_empty() && !self.query.trim().is_empty() {
                    ui.label("No results.");
                }
            });
        });

        egui::TopBottomPanel::bottom("tilik_footer").show(&mut ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::ProgressBar::new(self.progress as f32)
                        .text(if self.indexing {
                            format!("Indexing… {:.0}%", self.progress * 100.0)
                        } else {
                            "Index up to date".into()
                        }),
                );
                if !self.status.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.status);
                }
            });
        });
    }
}

/// Background worker: owns the index, searcher and watcher; the UI never
/// blocks on it.
fn worker(
    cmd_tx: mpsc::Sender<Cmd>,
    cmd_rx: mpsc::Receiver<Cmd>,
    ui_tx: mpsc::Sender<Msg>,
    index_dir: std::path::PathBuf,
) {
    let indexer = match Indexer::open_or_create(&index_dir) {
        Ok(x) => x,
        Err(e) => {
            let _ = ui_tx.send(Msg::Error(e.to_string()));
            return;
        }
    };
    let searcher = match Searcher::new(indexer.index()) {
        Ok(x) => x,
        Err(e) => {
            let _ = ui_tx.send(Msg::Error(e.to_string()));
            return;
        }
    };
    let mut _watcher: Option<Watcher> = None;
    let mut roots: Vec<String> = Vec::new();

    for cmd in cmd_rx {
        match cmd {
            Cmd::Search(q, reply) => {
                let hits = searcher.search(&q, SEARCH_LIMIT).unwrap_or_default();
                let _ = reply.send(hits);
            }
            Cmd::SetRoots(new_roots) => {
                _watcher = None; // stops the old watcher + its thread
                roots = new_roots;
                let (wtx, wrx) = mpsc::channel::<Vec<String>>();
                match watch(&roots, wtx) {
                    Ok(w) => _watcher = Some(w),
                    Err(e) => {
                        let _ = ui_tx.send(Msg::Error(e.to_string()));
                    }
                }
                let fwd_tx = cmd_tx.clone();
                let fwd_roots = roots.clone();
                std::thread::spawn(move || {
                    for batch in wrx {
                        let affected: Vec<String> = fwd_roots
                            .iter()
                            .filter(|r| {
                                batch.iter().any(|p| {
                                    p.to_lowercase().starts_with(&r.to_lowercase())
                                })
                            })
                            .cloned()
                            .collect();
                        if !affected.is_empty() {
                            let _ = fwd_tx.send(Cmd::ReindexRoots(affected));
                        }
                    }
                });
                // Rebuild the shared index from the full folder list so added
                // folders are cumulative and removed folders are cleaned.
                let _ = cmd_tx.send(Cmd::Reindex);
            }
            Cmd::Reindex => reindex_all(&indexer, &roots, &ui_tx),
            Cmd::ReindexRoots(rs) => {
                for r in &rs {
                    reindex_one(&indexer, r, &ui_tx);
                }
                let _ = ui_tx.send(Msg::Done);
            }
        }
    }
}

fn reindex_all(indexer: &Indexer, roots: &[String], ui_tx: &mpsc::Sender<Msg>) {
    let p = indexer.progress();
    let tx = ui_tx.clone();
    let poller = std::thread::spawn(move || {
        while p.ratio() < 1.0 {
            let _ = tx.send(Msg::Progress(p.ratio()));
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    if let Err(e) = indexer.index_directories(roots) {
        let _ = ui_tx.send(Msg::Error(e.to_string()));
    }
    let _ = poller.join();
    let _ = ui_tx.send(Msg::Done);
}

fn reindex_one(indexer: &Indexer, root: &str, ui_tx: &mpsc::Sender<Msg>) {
    let p = indexer.progress();
    let tx = ui_tx.clone();
    let poller = std::thread::spawn(move || {
        while p.ratio() < 1.0 {
            let _ = tx.send(Msg::Progress(p.ratio()));
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    if let Err(e) = indexer.index_directory(Path::new(root)) {
        let _ = ui_tx.send(Msg::Error(e.to_string()));
    }
    let _ = poller.join();
}

fn fmt_bytes(n: u64) -> String {
    const U: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1024.0 && i < 3 {
        f /= 1024.0;
        i += 1;
    }
    format!("{f:.1} {}", U[i])
}

/// Unix seconds → "YYYY-MM-DD HH:MM:SS" (civil-from-days, no chrono).
fn format_ts(ts: i64) -> String {
    if ts < 0 {
        return "unknown".into();
    }
    let z = ts / 86400 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 3652400) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let rem = ts % 86400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Tilik",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 480.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
