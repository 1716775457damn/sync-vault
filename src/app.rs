use crate::state::{default_excludes, Config, ExcludeSet, Store};
use crate::syncer::{fmt_bytes, full_sync, sync_file, SyncEvent};
use crate::watcher;
use chrono::Local;
use eframe::egui;
use egui::{Color32, RichText, ScrollArea, TextEdit};
use notify::RecommendedWatcher;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

const MAX_LOG: usize = 500;

pub struct App {
    cfg: Config,
    running: bool,
    paused: bool,
    store: Arc<Mutex<Store>>,
    log: VecDeque<(Color32, String)>,
    log_filter: String,
    log_filter_lc: String,
    log_errors_only: bool,         // quick filter: show only error lines
    filtered_cache: Vec<usize>,    // cached indices into log, rebuilt on change
    filtered_dirty: bool,          // true when log or filter changed
    dst_error: Option<String>,
    progress: Option<usize>,
    session_copied: usize,
    session_bytes: u64,
    stats_str: String,
    stats_key: (u64, u64, String),
    event_rx: Option<Receiver<SyncEvent>>,
    event_tx: Option<Sender<SyncEvent>>,
    file_rx: Option<Receiver<Vec<PathBuf>>>,
    _watcher: Option<RecommendedWatcher>,
    show_excludes: bool,
    new_exclude: String,
    src_error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        let cfg = Config::load();
        let cfg = if cfg.excludes.is_empty() {
            Config { excludes: default_excludes(), ..cfg }
        } else { cfg };
        Self {
            cfg,
            running: false,
            paused: false,
            store: Arc::new(Mutex::new(Store::load())),
            log: VecDeque::new(),
            log_filter: String::new(),
            log_filter_lc: String::new(),
            log_errors_only: false,
            filtered_cache: Vec::new(),
            filtered_dirty: true,
            dst_error: None,
            progress: None,
            session_copied: 0,
            session_bytes: 0,
            stats_str: String::new(),
            stats_key: (0, 0, String::new()),
            event_rx: None,
            event_tx: None,
            file_rx: None,
            _watcher: None,
            show_excludes: false,
            new_exclude: String::new(),
            src_error: None,
        }
    }
}

impl App {
    fn push_log(&mut self, color: Color32, msg: String) {
        let ts = Local::now().format("%H:%M:%S").to_string();
        if self.log.len() >= MAX_LOG { self.log.pop_front(); }
        self.log.push_back((color, format!("[{}] {}", ts, msg)));
    }

    fn start(&mut self) {
        let src = PathBuf::from(&self.cfg.src);
        let dst = PathBuf::from(&self.cfg.dst);
        if !src.exists() {
            self.src_error = Some(format!("路径不存在: {}", self.cfg.src));
            return;
        }
        // Guard: src == dst would cause infinite loop
        if src == dst {
            self.src_error = Some("源目录和目标目录不能相同".to_string());
            return;
        }
        // Guard: dst inside src would cause recursive copy
        if dst.starts_with(&src) {
            self.dst_error = Some("目标目录不能是源目录的子目录".to_string());
            return;
        }
        self.src_error = None;
        self.dst_error = None;
        if !dst.exists() {
            if std::fs::create_dir_all(&dst).is_err() {
                self.push_log(Color32::RED, format!("❌ 无法创建目标路径: {}", self.cfg.dst));
                return;
            }
        }
        self.cfg.save();

        let (etx, erx) = mpsc::channel::<SyncEvent>();
        let (ftx, frx) = mpsc::channel::<Vec<PathBuf>>();
        self.event_rx = Some(erx);
        self.event_tx = Some(etx.clone());
        self.file_rx = Some(frx);

        match watcher::start(src.clone(), ftx) {
            Ok(w) => { self._watcher = Some(w); }
            Err(e) => { self.push_log(Color32::RED, format!("❌ 监听失败: {e}")); return; }
        }

        let store = self.store.clone();
        let delete_removed = self.cfg.delete_removed;
        let excludes = self.cfg.excludes.clone();
        std::thread::spawn(move || {
            let mut st = store.lock().unwrap();
            full_sync(&src, &dst, &mut st, delete_removed, &excludes, &etx);
            st.flush_now();
        });

        self.running = true;
        self.push_log(Color32::GREEN, format!("✅ 开始监控: {} → {}", self.cfg.src, self.cfg.dst));
    }

    fn stop(&mut self) {
        self._watcher = None;
        self.event_tx = None;
        self.event_rx = None;
        self.file_rx = None;
        self.running = false;
        self.progress = None;
        self.push_log(Color32::GRAY, format!(
            "⏹ 已停止（本次同步 {} 个文件 {}）",
            self.session_copied, fmt_bytes(self.session_bytes)
        ));
        self.session_copied = 0;
        self.session_bytes = 0;
        self.store.lock().unwrap().flush_now();
    }

    fn resync(&mut self) {
        if !self.running { return; }
        let src = PathBuf::from(&self.cfg.src);
        let dst = PathBuf::from(&self.cfg.dst);
        let store = self.store.clone();
        let delete_removed = self.cfg.delete_removed;
        let excludes = self.cfg.excludes.clone();
        if let Some(ref etx) = self.event_tx {
            let etx = etx.clone();
            std::thread::spawn(move || {
                let mut st = store.lock().unwrap();
                full_sync(&src, &dst, &mut st, delete_removed, &excludes, &etx);
                st.flush_now();
            });
            self.push_log(Color32::from_rgb(100, 180, 255), "🔄 手动触发全量同步…".to_string());
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.store.lock().unwrap().flush_now();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain sync events
        let mut entries: Vec<(Color32, String)> = Vec::new();
        if let Some(ref rx) = self.event_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    SyncEvent::Copied { ref rel, bytes } => {
                        self.session_copied += 1;
                        self.session_bytes += bytes;
                        entries.push((Color32::from_rgb(100, 220, 100),
                            format!("📋 已同步  {}  ({})", rel, fmt_bytes(bytes))));
                    }
                    SyncEvent::Deleted { ref rel } =>
                        entries.push((Color32::from_rgb(220, 120, 80),
                            format!("🗑 已删除  {}", rel))),
                    SyncEvent::Error { ref rel, ref err } =>
                        entries.push((Color32::RED,
                            format!("❌ 错误  {}  {}", rel, err))),
                    SyncEvent::Progress { scanned, total } => {
                        self.progress = if scanned >= total && total > 0 { None }
                                        else { Some(scanned) };
                    }
                }
            }
        }
        if !entries.is_empty() {
            for (c, m) in entries { self.push_log(c, m); }
            self.filtered_dirty = true;
            ctx.request_repaint();
        }

        // Drain file change events — skip if paused
        if let Some(ref frx) = self.file_rx {
            let mut changed: Vec<PathBuf> = Vec::new();
            while let Ok(paths) = frx.try_recv() {
                if !self.paused { changed.extend(paths); }
            }
            if !changed.is_empty() {
                let src = PathBuf::from(&self.cfg.src);
                let dst = PathBuf::from(&self.cfg.dst);
                let store = self.store.clone();
                let excludes = self.cfg.excludes.clone();
                if let Some(ref etx) = self.event_tx {
                    let etx = etx.clone();
                    changed.sort(); changed.dedup();
                    std::thread::spawn(move || {
                        let ex = ExcludeSet::new(&excludes); // build once for the whole batch
                        let mut st = store.lock().unwrap();
                        for path in changed {
                            sync_file(&path, &src, &dst, &mut st, &ex, &etx);
                        }
                        st.flush_if_needed();
                    });
                }
                ctx.request_repaint();
            }
        }

        // try_lock: never block UI thread; also rebuild cached stats string
        if let Ok(mut st) = self.store.try_lock() {
            st.flush_if_needed();
            let last = st.state.last_sync
                .map(|t| t.format("%m/%d %H:%M:%S").to_string())
                .unwrap_or_default();
            let key = (st.state.total_synced, st.state.total_bytes, last);
            if key != self.stats_key {
                self.stats_str = format!(
                    "累计同步 {} 个文件  {}  |  上次: {}",
                    key.0, fmt_bytes(key.1),
                    if key.2.is_empty() { "从未".to_string() } else { key.2.clone() }
                );
                self.stats_key = key;
            }
        }

        // Config panel
        egui::TopBottomPanel::top("config")
            .frame(egui::Frame::side_top_panel(&ctx.style())
                .inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 8 }))
            .show(ctx, |ui| {
            // Source row
            ui.horizontal(|ui| {
                ui.label(RichText::new("源目录").color(egui::Color32::from_rgb(140, 155, 175)).size(12.0));
                let resp = ui.add(
                    TextEdit::singleline(&mut self.cfg.src)
                        .desired_width(240.0)
                        .hint_text("要监控的文件夹…")
                        .font(egui::TextStyle::Body)
                        .text_color(if self.src_error.is_some() {
                            Color32::from_rgb(248, 113, 113)
                        } else {
                            Color32::from_rgb(220, 228, 240)
                        }),
                );
                if resp.changed() { self.src_error = None; }
                if ui.add(egui::Button::new("📁").min_size(egui::vec2(28.0, 28.0))).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.cfg.src = p.to_string_lossy().replace('\\', "/");
                        self.src_error = None;
                    }
                }
                if let Some(ref err) = self.src_error {
                    ui.label(RichText::new(format!("⚠  {err}")).color(Color32::from_rgb(248, 113, 113)).size(11.5));
                }
            });
            // Dest row
            ui.horizontal(|ui| {
                ui.label(RichText::new("目标目录").color(egui::Color32::from_rgb(140, 155, 175)).size(12.0));
                let resp = ui.add(
                    TextEdit::singleline(&mut self.cfg.dst)
                        .desired_width(240.0)
                        .hint_text("备份到哪里…")
                        .font(egui::TextStyle::Body)
                        .text_color(if self.dst_error.is_some() {
                            Color32::from_rgb(248, 113, 113)
                        } else {
                            Color32::from_rgb(220, 228, 240)
                        }),
                );
                if resp.changed() { self.dst_error = None; }
                if ui.add(egui::Button::new("📁").min_size(egui::vec2(28.0, 28.0))).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.cfg.dst = p.to_string_lossy().replace('\\', "/");
                        self.dst_error = None;
                    }
                }
                if let Some(ref err) = self.dst_error {
                    ui.label(RichText::new(format!("⚠  {err}")).color(Color32::from_rgb(248, 113, 113)).size(11.5));
                }
            });
            // Controls row
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.cfg.delete_removed, "同步删除")
                    .on_hover_text("源目录删除的文件，目标目录也同步删除");
                ui.add(egui::Separator::default().vertical().spacing(4.0));
                if ui.small_button(
                    if self.show_excludes { "▲ 排除规则" } else { "▼ 排除规则" }
                ).clicked() {
                    self.show_excludes = !self.show_excludes;
                }
                ui.add_space(8.0);
                if self.running {
                    if ui.add(
                        egui::Button::new("⏹  停止")
                            .fill(Color32::from_rgb(127, 29, 29))
                            .min_size(egui::vec2(64.0, 28.0))
                    ).clicked() { self.stop(); }

                    let (pause_label, pause_fill) = if self.paused {
                        ("▶  恢复", Color32::from_rgb(92, 78, 14))
                    } else {
                        ("⏸  暂停", Color32::from_rgb(38, 42, 54))
                    };
                    if ui.add(
                        egui::Button::new(pause_label)
                            .fill(pause_fill)
                            .min_size(egui::vec2(64.0, 28.0))
                    ).on_hover_text(if self.paused { "恢复监听文件变动" } else { "暂停监听，不停止已运行的同步" })
                        .clicked() { self.paused = !self.paused; }

                    if ui.add(
                        egui::Button::new("🔄  立即同步")
                            .fill(Color32::from_rgb(6, 78, 59))
                            .min_size(egui::vec2(80.0, 28.0))
                    ).on_hover_text("触发一次全量增量同步").clicked() { self.resync(); }

                    ui.spinner();
                    let (status_text, status_color) = if self.paused {
                        ("已暂停", Color32::from_rgb(251, 191, 36))
                    } else {
                        ("监控中…", Color32::from_rgb(52, 211, 153))
                    };
                    ui.label(RichText::new(status_text).color(status_color).size(12.0));
                } else {
                    if ui.add(
                        egui::Button::new("▶  开始同步")
                            .fill(Color32::from_rgb(6, 78, 59))
                            .min_size(egui::vec2(88.0, 28.0))
                    ).clicked() { self.start(); }
                }
            });

            if self.show_excludes {
                ui.add(egui::Separator::default().spacing(6.0));
                ui.label(RichText::new("排除规则（文件名、目录名或 *.ext）")
                    .color(Color32::from_rgb(120, 135, 155)).size(11.5));
                ScrollArea::vertical().id_salt("excl").max_height(110.0).show(ui, |ui| {
                    let mut to_remove: Option<usize> = None;
                    egui::Grid::new("excludes").num_columns(2).spacing(egui::vec2(6.0, 3.0)).show(ui, |ui| {
                        for (i, pat) in self.cfg.excludes.iter().enumerate() {
                            ui.label(RichText::new(pat).monospace().size(12.0)
                                .color(Color32::from_rgb(180, 195, 215)));
                            if ui.small_button("✕").clicked() { to_remove = Some(i); }
                            ui.end_row();
                        }
                    });
                    if let Some(i) = to_remove { self.cfg.excludes.remove(i); self.cfg.save(); }
                });
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut self.new_exclude)
                        .desired_width(180.0)
                        .hint_text("node_modules 或 *.log")
                        .font(egui::TextStyle::Small));
                    if ui.add(
                        egui::Button::new("+ 添加")
                            .fill(Color32::from_rgb(30, 58, 50))
                    ).clicked() && !self.new_exclude.is_empty() {
                        self.cfg.excludes.push(self.new_exclude.drain(..).collect());
                        self.cfg.save();
                    }
                });
            }
        });

        // Status bar
        egui::TopBottomPanel::bottom("stats")
            .frame(egui::Frame::side_top_panel(&ctx.style())
                .inner_margin(egui::Margin { left: 14, right: 14, top: 5, bottom: 5 }))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(scanned) = self.progress {
                    ui.spinner();
                    ui.label(RichText::new(format!("已扫描 {} 个文件", scanned))
                        .size(11.5).color(Color32::from_rgb(96, 165, 250)));
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                }
                if self.session_copied > 0 {
                    ui.label(RichText::new(format!(
                        "本次: {} 个文件  {}", self.session_copied, fmt_bytes(self.session_bytes)
                    )).size(11.5).color(Color32::from_rgb(52, 211, 153)));
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                }
                ui.label(RichText::new(&self.stats_str).size(11.5).color(Color32::from_rgb(100, 116, 139)));
            });
        });

        // Log panel with cached filter
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("同步日志").strong());
                ui.label(RichText::new(format!("({} 条)", self.log.len())).small().color(Color32::GRAY));
                let filter_changed = {
                    let new_lc = self.log_filter.to_lowercase();
                    if new_lc != self.log_filter_lc {
                        self.log_filter_lc = new_lc;
                        self.filtered_dirty = true;
                        true
                    } else { false }
                };
                let _ = filter_changed;
                ui.add(egui::TextEdit::singleline(&mut self.log_filter)
                    .hint_text("过滤日志…")
                    .desired_width(110.0));
                if !self.log_filter.is_empty() && ui.small_button("✕").clicked() {
                    self.log_filter.clear();
                    self.log_filter_lc.clear();
                    self.filtered_dirty = true;
                }
                // Quick error filter toggle
                let err_col = if self.log_errors_only { Color32::RED } else { Color32::DARK_GRAY };
                if ui.small_button(RichText::new("❌ 错误").color(err_col))
                    .on_hover_text("只显示错误日志")
                    .clicked()
                {
                    self.log_errors_only = !self.log_errors_only;
                    self.filtered_dirty = true;
                }
                if ui.small_button("清空").clicked() {
                    self.log.clear();
                    self.filtered_dirty = true;
                }
            });
            ui.separator();

            // Rebuild filter cache only when dirty
            if self.filtered_dirty {
                self.filtered_dirty = false;
                self.filtered_cache = self.log.iter().enumerate()
                    .filter(|(_, (color, msg))| {
                        if self.log_errors_only && *color != Color32::RED { return false; }
                        if !self.log_filter_lc.is_empty() {
                            return msg.to_lowercase().contains(&self.log_filter_lc);
                        }
                        true
                    })
                    .map(|(i, _)| i)
                    .collect();
            }

            let n = self.filtered_cache.len();
            let is_filtered = !self.log_filter_lc.is_empty() || self.log_errors_only;
            if n == 0 && self.log.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("设置源目录和目标目录后点击「开始同步」").color(Color32::GRAY));
                });
            } else if n == 0 {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("无匹配的日志").color(Color32::GRAY));
                });
            } else {
                ScrollArea::vertical().stick_to_bottom(!is_filtered).auto_shrink(false)
                    .show_rows(ui, 16.0, n, |ui, range| {
                        for i in range {
                            let log_idx = self.filtered_cache[i];
                            let (color, msg) = &self.log[log_idx];
                            ui.label(RichText::new(msg).small().color(*color).monospace());
                        }
                    });
            }
        });
    }
}
