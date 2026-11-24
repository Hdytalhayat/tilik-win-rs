#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use tilik_core::config::Config;
use tilik_core::extractor;
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
    show_folders_modal: bool,
    _platform: Option<platform::windows::Platform>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Modern dark surface hierarchy.
        let base = egui::Color32::from_rgb(11, 14, 20);
        let surface = egui::Color32::from_rgb(20, 25, 35);
        let elevated = egui::Color32::from_rgb(28, 35, 48);
        let border = egui::Color32::from_rgb(42, 51, 68);
        let text = egui::Color32::from_rgb(242, 245, 250);
        let accent = egui::Color32::from_rgb(66, 133, 244);

        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(text);
        visuals.window_fill = base;
        visuals.panel_fill = base;
        visuals.faint_bg_color = surface;
        visuals.extreme_bg_color = egui::Color32::from_rgb(6, 7, 10);
        visuals.widgets.noninteractive.bg_fill = surface;
        visuals.widgets.noninteractive.weak_bg_fill = surface;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text);
        visuals.widgets.inactive.bg_fill = elevated;
        visuals.widgets.inactive.weak_bg_fill = elevated;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, border);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, text);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(36, 45, 62);
        visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(36, 45, 62);
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, accent);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, text);
        visuals.widgets.active.bg_fill = accent;
        visuals.widgets.active.weak_bg_fill = accent;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, accent);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
        visuals.selection.bg_fill = accent.linear_multiply(0.35);
        visuals.selection.stroke = egui::Stroke::new(1.0_f32, accent);
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
        style.visuals.window_corner_radius = egui::CornerRadius::same(8);
        cc.egui_ctx.set_style(style);

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
            show_folders_modal: false,
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

    fn commit_roots(&mut self) {
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        platform::windows::publish_context(&ctx);
        if platform::windows::take_focus_request() {
            self.focused = false;
        }

        // X button hides to tray.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }

        // Drain worker messages.
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
        if self.indexing {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Escape closes modal first, then hides app.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_folders_modal {
                self.show_folders_modal = false;
                self.focused = false;
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }

        // Debounced search.
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

        // Collect search replies.
        if let Some(rx) = self.reply_rx.as_ref() {
            if let Ok(hits) = rx.try_recv() {
                if self.pending_query == self.query.trim() {
                    self.results = hits;
                    self.selected = 0;
                    self.reply_rx = None;
                }
            }
        }
        if self.selected >= self.results.len() && !self.results.is_empty() {
            self.selected = self.results.len().saturating_sub(1);
        }

        // Keyboard navigation (disabled while modal is open).
        if !self.show_folders_modal && !self.results.is_empty() {
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

        // Palette
        let base = egui::Color32::from_rgb(11, 14, 20);
        let surface = egui::Color32::from_rgb(20, 25, 35);
        let elevated = egui::Color32::from_rgb(28, 35, 48);
        let border = egui::Color32::from_rgb(42, 51, 68);
        let text = egui::Color32::from_rgb(242, 245, 250);
        let muted = egui::Color32::from_rgb(142, 155, 174);
        let accent = egui::Color32::from_rgb(66, 133, 244);
        let success = egui::Color32::from_rgb(52, 211, 153);
        let danger = egui::Color32::from_rgb(248, 113, 113);

        // ---------- Top search bar ----------
        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame::new().fill(base).inner_margin(12.0))
            .show(ctx, |ui| {
                // Gambar tombol Folder di kanan dulu supaya search box mengisi sisa lebar dengan aman.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let folder_label = format!("Folder ({})", self.roots.len());
                    let folder_button = if self.roots.is_empty() {
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new(folder_label)
                                    .strong()
                                    .color(egui::Color32::BLACK),
                            )
                            .fill(egui::Color32::from_rgb(245, 158, 11)),
                        )
                    } else {
                        ui.add(
                            egui::Button::new(egui::RichText::new(folder_label).color(text))
                                .fill(elevated),
                        )
                    };
                    if folder_button.clicked() {
                        self.show_folders_modal = !self.show_folders_modal;
                    }

                    // Kotak pencarian mengisi sisa lebar di kiri
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        egui::Frame::new()
                            .fill(elevated)
                            .stroke(egui::Stroke::new(1.0_f32, border))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if !self.query.is_empty()
                                        && ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new("×").size(16.0).color(muted),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                    {
                                        self.query.clear();
                                        self.results.clear();
                                        self.last_searched.clear();
                                        self.focused = false;
                                    }

                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("Cari").color(muted));
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(&mut self.query)
                                                .id(egui::Id::new("main_search_input"))
                                                .hint_text("Ketik nama file, ekstensi (.pdf, .jpg), atau isi dokumen...")
                                                .desired_width(ui.available_width())
                                                .frame(false),
                                        );
                                        if !self.focused && !self.show_folders_modal {
                                            resp.request_focus();
                                            self.focused = true;
                                        }
                                    });
                                });
                            });
                    });
                });
            });

        // ---------- Footer (gambar sebelum CentralPanel agar docking bersih) ----------
        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(base)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(egui::Stroke::new(1.0_f32, border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.indexing {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(format!(
                                "Mengindeks... {:.0}%",
                                self.progress * 100.0
                            ))
                            .color(muted),
                        );
                    } else {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, success);
                        ui.label(
                            egui::RichText::new(format!(
                                "Indeks siap · {} folder",
                                self.roots.len()
                            ))
                            .color(muted),
                        );
                    }
                    if !self.status.is_empty() {
                        ui.label(egui::RichText::new(&self.status).color(danger));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Arrow Atas/Bawah Pilih  ·  Enter Buka  ·  Ctrl+Enter Lokasi  ·  Esc Sembunyikan",
                            )
                            .size(11.0)
                            .color(muted),
                        );
                    });
                });
            });

        // ---------- Main workspace ----------
        if self.query.trim().is_empty() {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(base).inner_margin(24.0))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(egui::RichText::new("Tilik").size(34.0).strong().color(text));
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Cari file & konten dokumen secara instan")
                                .color(muted),
                        );
                        ui.add_space(20.0);

                        // Contoh kata kunci terpusat
                        let chips = [".pdf", ".jpg", ".txt", "surat"];
                        let total_chip_w = chips.len() as f32 * 64.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(total_chip_w, 32.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                for chip in chips {
                                    if ui.button(chip).clicked() {
                                        self.query = chip.to_string();
                                        self.focused = false;
                                    }
                                }
                            },
                        );

                        if self.roots.is_empty() {
                            ui.add_space(20.0);
                            if ui
                                .add(egui::Button::new("Tambah Folder Pertama").fill(accent))
                                .clicked()
                            {
                                self.show_folders_modal = true;
                            }
                        }
                    });
                });
        } else {
            egui::SidePanel::left("results")
                .resizable(true)
                .default_width(340.0)
                .min_width(240.0)
                .frame(
                    egui::Frame::new()
                        .fill(surface)
                        .inner_margin(12.0)
                        .stroke(egui::Stroke::new(1.0_f32, border)),
                )
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{} hasil ditemukan", self.results.len()))
                            .size(12.0)
                            .color(muted),
                    );
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, h) in self.results.iter().enumerate() {
                            let selected = i == self.selected;
                            let card_fill = if selected { elevated } else { surface };
                            let stroke_color = if selected { accent } else { border };
                            let frame_resp = egui::Frame::new()
                                .fill(card_fill)
                                .stroke(egui::Stroke::new(1.0_f32, stroke_color))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        let badge_color = ext_color(&h.extension);
                                        egui::Frame::new()
                                            .fill(badge_color)
                                            .corner_radius(egui::CornerRadius::same(4))
                                            .inner_margin(egui::Margin::symmetric(5, 2))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    egui::RichText::new(
                                                        h.extension.to_uppercase(),
                                                    )
                                                    .size(10.0)
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                                );
                                            });
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&h.filename)
                                                    .strong()
                                                    .color(text),
                                            )
                                            .truncate(),
                                        );
                                    });
                                    let parent = Path::new(&h.path)
                                        .parent()
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(parent).size(11.5).color(muted),
                                        )
                                        .truncate(),
                                    );
                                });
                            if frame_resp.response.interact(egui::Sense::click()).clicked() {
                                self.selected = i;
                            }
                            ui.add_space(4.0);
                        }
                    });
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(base).inner_margin(12.0))
                .show(ctx, |ui| {
                    if let Some(h) = self.results.get(self.selected).cloned() {
                        // Header Card
                        egui::Frame::new()
                            .fill(surface)
                            .stroke(egui::Stroke::new(1.0_f32, border))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let badge_color = ext_color(&h.extension);
                                    egui::Frame::new()
                                        .fill(badge_color)
                                        .corner_radius(egui::CornerRadius::same(4))
                                        .inner_margin(egui::Margin::symmetric(5, 2))
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(h.extension.to_uppercase())
                                                    .size(10.0)
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                            );
                                        });
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&h.filename)
                                                .size(18.0)
                                                .strong()
                                                .color(text),
                                        )
                                        .wrap(),
                                    );
                                });
                                ui.add_space(2.0);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&h.path).size(11.5).color(muted),
                                    )
                                    .wrap(),
                                );
                            });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new("Buka File").fill(accent))
                                .clicked()
                            {
                                self.open_selected(false);
                            }
                            if ui
                                .add(egui::Button::new("Buka Lokasi").fill(elevated))
                                .clicked()
                            {
                                self.open_selected(true);
                            }
                            if ui
                                .add(egui::Button::new("Salin Path").fill(elevated))
                                .clicked()
                            {
                                ctx.copy_text(h.path.clone());
                            }
                        });

                        ui.add_space(8.0);
                        // Metadata Card
                        egui::Frame::new()
                            .fill(surface)
                            .stroke(egui::Stroke::new(1.0_f32, border))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::Grid::new("meta")
                                    .num_columns(2)
                                    .spacing(egui::vec2(16.0, 6.0))
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new("Ukuran").color(muted));
                                        ui.label(fmt_bytes(
                                            std::fs::metadata(&h.path)
                                                .map(|m| m.len())
                                                .unwrap_or(0),
                                        ));
                                        ui.end_row();
                                        ui.label(egui::RichText::new("Diubah").color(muted));
                                        ui.label(format_ts(h.modified_ts));
                                        ui.end_row();
                                        ui.label(egui::RichText::new("Lokasi").color(muted));
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&h.path).color(text),
                                            )
                                            .wrap(),
                                        );
                                        ui.end_row();
                                    });
                            });

                        ui.add_space(8.0);
                        // Content Preview Card
                        egui::Frame::new()
                            .fill(elevated)
                            .stroke(egui::Stroke::new(1.0_f32, border))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(ui.available_height());
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let ext = h.extension.to_ascii_lowercase();
                                    let is_video = extractor::is_video_extension(&ext);
                                    if is_video {
                                        ui.label(
                                            egui::RichText::new(
                                                "Pratinjau teks tidak tersedia untuk file video.",
                                            )
                                            .color(muted),
                                        );
                                    } else if !extractor::is_text_preview_available(&ext, &h.snippet) {
                                        ui.label(
                                            egui::RichText::new(
                                                "Pratinjau teks tidak tersedia untuk file ini. Klik \"Buka File\" untuk melihatnya.",
                                            )
                                            .color(muted),
                                        );
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(
                                                    h.snippet.replace("<b>", "").replace("</b>", ""),
                                                )
                                                .monospace()
                                                .color(text),
                                            )
                                            .wrap(),
                                        );
                                    }
                                });
                            });
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Tidak ada file yang cocok dengan pencarian Anda.",
                                )
                                .color(muted),
                            );
                        });
                    }
                });
        }

        // ---------- Modal folder (gambar paling akhir agar di atas semua panel) ----------
        if self.show_folders_modal {
            // Backdrop redup agar fokus ke modal
            let screen = ctx.screen_rect();
            egui::Area::new(egui::Id::new("modal_backdrop"))
                .fixed_pos(screen.min)
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    ui.allocate_response(screen.size(), egui::Sense::click());
                    ui.painter()
                        .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(150));
                });

            egui::Window::new("Folder Terindeks")
                .order(egui::Order::Foreground)
                .collapsible(false)
                .resizable(false)
                .fixed_size([480.0, 360.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Tilik memantau dan mencari nama file maupun isi dokumen di dalam folder berikut:",
                        )
                        .color(muted),
                    );
                    ui.add_space(6.0);
                    if ui
                        .add(egui::Button::new("+ Pilih Folder...").fill(accent))
                        .clicked()
                    {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            let t = folder.to_string_lossy().replace('/', "\\");
                            if !t.is_empty() && !self.roots.contains(&t) {
                                self.roots.push(t);
                            }
                            self.commit_roots();
                        }
                    }
                    ui.add_space(6.0);

                    egui::ScrollArea::vertical()
                        .max_height(190.0)
                        .show(ui, |ui| {
                            if self.roots.is_empty() {
                                ui.label(
                                    egui::RichText::new(
                                        "Belum ada folder terdaftar. Klik \"+ Pilih Folder...\" di atas.",
                                    )
                                    .color(muted),
                                );
                            }
                            let roots = self.roots.clone();
                            for (i, r) in roots.iter().enumerate() {
                                egui::Frame::new()
                                    .fill(elevated)
                                    .stroke(egui::Stroke::new(1.0_f32, border))
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                    .show(ui, |ui| {
                                        ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui.small_button("Hapus").clicked() {
                                                        self.roots.remove(i);
                                                        self.commit_roots();
                                                    }
                                                    ui.with_layout(
                                                        egui::Layout::left_to_right(egui::Align::Center),
                                                        |ui| {
                                                            ui.add(
                                                                egui::Label::new(
                                                                    egui::RichText::new(r).color(text),
                                                                )
                                                                .truncate(),
                                                            );
                                                        },
                                                    );
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                        });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_root)
                                .hint_text("Atau ketik path folder manual (C:\\...)")
                                .desired_width(320.0),
                        );
                        if ui.button("Tambah").clicked() {
                            let t = self.new_root.trim().replace('/', "\\");
                            if !t.is_empty() && !self.roots.contains(&t) {
                                self.roots.push(t);
                            }
                            self.new_root.clear();
                            self.commit_roots();
                        }
                    });
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Selesai").clicked() {
                            self.show_folders_modal = false;
                            self.focused = false;
                        }
                    });
                });
        }
    }
}

fn ext_color(ext: &str) -> egui::Color32 {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => egui::Color32::from_rgb(239, 68, 68),
        "doc" | "docx" => egui::Color32::from_rgb(59, 130, 246),
        "xls" | "xlsx" | "csv" => egui::Color32::from_rgb(34, 197, 94),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "ico" => egui::Color32::from_rgb(168, 85, 247),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "3gp" => egui::Color32::from_rgb(249, 115, 22),
        "txt" | "md" | "rs" | "py" | "js" | "ts" | "json" => egui::Color32::from_rgb(148, 163, 184),
        _ => egui::Color32::from_rgb(100, 116, 139),
    }
}

#[allow(dead_code)]
fn status_label(indexing: bool, progress: f64) -> String {
    if indexing {
        format!("Mengindeks... {:.0}%", progress * 100.0)
    } else {
        "Index up to date".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_indexing() {
        assert_eq!(status_label(true, 0.25), "Mengindeks... 25%");
        assert_eq!(status_label(false, 1.0), "Index up to date");
    }

    #[test]
    fn video_extensions_are_classified() {
        assert!(tilik_core::extractor::is_video_extension("mp4"));
        assert!(tilik_core::extractor::is_video_extension("MKV"));
        assert!(!tilik_core::extractor::is_video_extension("txt"));
    }

    #[test]
    fn preview_text_sanitization() {
        assert!(!tilik_core::extractor::is_text_preview_available("mp4", "some text"));
        assert!(!tilik_core::extractor::is_text_preview_available("txt", ""));
        assert!(!tilik_core::extractor::is_text_preview_available("txt", "bad\u{FFFD}text"));
        assert!(tilik_core::extractor::is_text_preview_available("txt", "clean text"));
    }
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
                _watcher = None;
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

fn main() -> eframe::Result {
    eframe::run_native(
        "Tilik",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([920.0, 580.0])
                .with_min_inner_size([680.0, 420.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}