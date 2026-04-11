use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub hash: String,
    pub size: u64,
    pub modified: DateTime<Local>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct SyncState {
    pub files: HashMap<String, FileRecord>,
    pub last_sync: Option<DateTime<Local>>,
    pub total_synced: u64,
    pub total_bytes: u64,
}

/// Persisted user configuration
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub src: String,
    pub dst: String,
    pub delete_removed: bool,
    pub excludes: Vec<String>, // glob-style name patterns to skip
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = config_path() {
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                let _ = std::fs::write(p, json);
            }
        }
    }
}

pub struct Store {
    pub state: SyncState,
    path: PathBuf,
    dirty: bool,
    last_save: Instant,
}

impl Store {
    pub fn load() -> Self {
        let path = state_path();
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { state, path, dirty: false, last_save: Instant::now() }
    }

    pub fn mark_dirty(&mut self) { self.dirty = true; }

    /// Flush if dirty and 3s have elapsed
    pub fn flush_if_needed(&mut self) {
        if self.dirty && self.last_save.elapsed().as_secs() >= 3 {
            self.flush_now();
        }
    }

    pub fn flush_now(&mut self) {
        if !self.dirty { return; }
        // Use BufWriter to avoid intermediate String allocation
        if let Ok(file) = std::fs::File::create(&self.path) {
            let _ = std::fs::create_dir_all(self.path.parent().unwrap());
            let _ = serde_json::to_writer(std::io::BufWriter::new(file), &self.state);
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }
}

fn state_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sync-vault")
        .join("state.json")
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?
        .join("sync-vault")
        .join("config.json"))
}

/// Pre-compiled exclude patterns for efficient matching
pub struct ExcludeSet {
    exact: std::collections::HashSet<String>,
    exts: Vec<String>,
}

impl ExcludeSet {
    pub fn new(excludes: &[String]) -> Self {
        let exact = excludes.iter()
            .filter(|p| !p.starts_with("*."))
            .cloned()
            .collect();
        let exts = excludes.iter()
            .filter_map(|p| p.strip_prefix("*.").map(|s| s.to_string()))
            .collect();
        Self { exact, exts }
    }

    pub fn matches(&self, rel: &str) -> bool {
        for segment in rel.split('/') {
            if self.exact.contains(segment) { return true; }
            for ext in &self.exts {
                // Avoid format! allocation: check suffix directly
                if segment.len() > ext.len() + 1
                    && segment.as_bytes()[segment.len() - ext.len() - 1] == b'.'
                    && segment.ends_with(ext.as_str())
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Check if any path segment matches an exclude pattern
#[allow(dead_code)]
pub fn is_excluded(rel: &str, excludes: &[String]) -> bool {
    ExcludeSet::new(excludes).matches(rel)
}

pub fn default_excludes() -> Vec<String> {
    vec![
        ".git".into(), ".svn".into(), ".hg".into(),
        "node_modules".into(), "__pycache__".into(),
        "target".into(), ".DS_Store".into(),
        "Thumbs.db".into(), "*.tmp".into(), "*.swp".into(),
    ]
}
