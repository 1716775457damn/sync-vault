use crate::state::{default_excludes, Config, Store};
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
    paused: bool,              // pause watcher events without stopping
    store: Arc<Mutex<Store>>,
    log: VecDeque<(Color32, String)>,
    log_filter: String,        // filter log by keyword
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
        self.src_error = None;
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
                        let mut st = store.lock().unwrap();
                        for path in changed {
                            sync_file(&path, &src, &dst, &mut st, &excludes, &etx);
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
        egui::TopBottomPanel::top("config").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("源目录:");
                let resp = ui.add(
                    TextEdit::singleline(&mut self.cfg.src)
                        .desired_width(220.0)
                        .hint_text("要监控的文件夹…")
                        .text_color(if self.src_error.is_some() {
                            Color32::from_rgb(255, 100, 100)
                        } else {
                            ui.visuals().text_color()
                        }),
                );
                if resp.changed() { self.src_error = None; }
                if ui.button("📁").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.cfg.src = p.to_string_lossy().replace('\\', "/");
                        self.src_error = None;
                    }
                }
            });
            if let Some(ref err) = self.src_error {
                ui.label(RichText::new(err).small().color(Color32::from_rgb(255, 100, 100)));
            }
            ui.horizontal(|ui| {
                ui.label("目标目录:");
                ui.add(TextEdit::singleline(&mut self.cfg.dst)
                    .desired_width(220.0).hint_text("备份到哪里…"));
                if ui.button("📁").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.cfg.dst = p.to_string_lossy().replace('\\', "/");
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.cfg.delete_removed, "同步删除")
                    .on_hover_text("源目录删除的文件，目标目录也同步删除");
                ui.separator();
                if ui.small_button(if self.show_excludes { "▲ 排除规则" } else { "▼ 排除规则" }).clicked() {
                    self.show_excludes = !self.show_excludes;
                }
                ui.add_space(8.0);
                if self.running {
                    if ui.button(RichText::new("⏹ 停止").color(Color32::RED)).clicked() { self.stop(); }
                    // Pause/resume toggle
                    let pause_label = if self.paused { "▶ 恢复" } else { "⏸ 暂停" };
                    let pause_color = if self.paused { Color32::YELLOW } else { Color32::from_rgb(200,200,200) };
                    if ui.button(RichText::new(pause_label).color(pause_color))
                        .on_hover_text(if self.paused { "恢复监听文件变动" } else { "暂停监听，不停止已运行的同步" })
                        .clicked() { self.paused = !self.paused; }
                    if ui.button("🔄 立即同步").on_hover_text("触发一次全量增量同步").clicked() { self.resync(); }
                    ui.spinner();
                    let status = if self.paused {
                        RichText::new("已暂停").color(Color32::YELLOW).small()
                    } else {
                        RichText::new("监控中…").color(Color32::GREEN).small()
                    };
                    ui.label(status);
                } else {
                    if ui.button(RichText::new("▶ 开始同步").color(Color32::GREEN)).clicked() { self.start(); }
                }
            });

            if self.show_excludes {
                ui.separator();
                ui.label(RichText::new("排除规则（文件名、目录名或 *.ext）").small().color(Color32::GRAY));
                let mut to_remove: Option<usize> = None;
                egui::Grid::new("excludes").num_columns(2).show(ui, |ui| {
                    for (i, pat) in self.cfg.excludes.iter().enumerate() {
                        ui.label(RichText::new(pat).monospace().small());
                        if ui.small_button("✕").clicked() { to_remove = Some(i); }
                        ui.end_row();
                    }
                });
                if let Some(i) = to_remove { self.cfg.excludes.remove(i); self.cfg.save(); }
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut self.new_exclude)
                        .desired_width(160.0).hint_text("node_modules 或 *.log"));
                    if ui.small_button("添加").clicked() && !self.new_exclude.is_empty() {
                        self.cfg.excludes.push(self.new_exclude.drain(..).collect());
                        self.cfg.save();
                    }
                });
            }
            ui.add_space(4.0);
        });

        // Status bar
        egui::TopBottomPanel::bottom("stats").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(scanned) = self.progress {
                    ui.spinner();
                    ui.label(RichText::new(format!("已扫描 {} 个文件", scanned))
                        .small().color(Color32::from_rgb(100, 180, 255)));
                    ui.separator();
                }
                if self.session_copied > 0 {
                    ui.label(RichText::new(format!(
                        "本次: {} 个文件 {}", self.session_copied, fmt_bytes(self.session_bytes)
                    )).small().color(Color32::from_rgb(100, 220, 100)));
                    ui.separator();
                }
                ui.label(RichText::new(&self.stats_str).small().color(Color32::GRAY));
            });
        });

        // Log panel with virtual scroll
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("同步日志").strong());
                ui.label(RichText::new(format!("({} 条)", self.log.len())).small().color(Color32::GRAY));
                ui.add(egui::TextEdit::singleline(&mut self.log_filter)
                    .hint_text("过滤日志…")
                    .desired_width(120.0));
                if !self.log_filter.is_empty() && ui.small_button("✕").clicked() {
                    self.log_filter.clear();
                }
                if ui.small_button("清空").clicked() { self.log.clear(); }
            });
            ui.separator();
            let filter_lc = self.log_filter.to_lowercase();
            let filtered: Vec<&(Color32, String)> = if filter_lc.is_empty() {
                self.log.iter().collect()
            } else {
                self.log.iter().filter(|(_, m)| m.to_lowercase().contains(&filter_lc)).collect()
            };
            let n = filtered.len();
            if n == 0 && self.log.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("设置源目录和目标目录后点击「开始同步」").color(Color32::GRAY));
                });
            } else if n == 0 {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("无匹配的日志").color(Color32::GRAY));
                });
            } else {
                ScrollArea::vertical().stick_to_bottom(filter_lc.is_empty()).auto_shrink(false)
                    .show_rows(ui, 16.0, n, |ui, range| {
                        for i in range {
                            let (color, msg) = filtered[i];
                            ui.label(RichText::new(msg).small().color(*color).monospace());
                        }
                    });
            }
        });
    }
}
