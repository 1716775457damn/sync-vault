use crate::state::{ExcludeSet, FileRecord, Store};
use anyhow::Result;
use chrono::Local;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

#[inline]
fn nfc_path(s: std::borrow::Cow<'_, str>) -> String {
    if s.is_ascii() { return s.into_owned(); }
    s.nfc().collect()
}

pub enum SyncEvent {
    Copied { rel: String, bytes: u64 },
    Deleted { rel: String },
    Error { rel: String, err: String },
    Progress { scanned: usize, total: usize },
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Three-stage fast-path:
/// Stage 1 — size differs          → must copy, skip hash
/// Stage 2 — size same, mtime same → unchanged, skip hash entirely
/// Stage 3 — size same, mtime diff → hash to confirm
fn scan_needed(
    src: &Path,
    store: &Store,
    excludes: &[String],
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) -> (Vec<(String, PathBuf, u64, String)>, std::collections::HashSet<String>) {
    let ex = ExcludeSet::new(excludes);
    let mut seen    = std::collections::HashSet::new();
    let mut scanned = 0usize;

    let mut need_hash:    Vec<(String, PathBuf, u64)> = Vec::new();
    let mut size_changed: Vec<(String, PathBuf, u64)> = Vec::new();

    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() { continue; }
        let abs = entry.path();
        let rel = match abs.strip_prefix(src) {
            Ok(r) => nfc_path(r.to_string_lossy()).replace('\\', "/"),
            Err(_) => continue,
        };

        scanned += 1;
        if scanned % 100 == 0 {
            let _ = tx.send(SyncEvent::Progress { scanned, total: 0 });
        }

        if ex.matches(&rel) { continue; }
        seen.insert(rel.clone());

        let meta = match entry.metadata().or_else(|_| std::fs::metadata(abs)) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();

        match store.state.files.get(&rel) {
            None => {
                need_hash.push((rel, abs.to_path_buf(), size));
            }
            Some(rec) => {
                if rec.size != size {
                    size_changed.push((rel, abs.to_path_buf(), size));
                } else {
                    let mtime = meta.modified()
                        .ok()
                        .map(|t| chrono::DateTime::<Local>::from(t).timestamp())
                        .unwrap_or(0);
                    if rec.modified.timestamp() == mtime { continue; }
                    need_hash.push((rel, abs.to_path_buf(), size));
                }
            }
        }
    }
    let _ = tx.send(SyncEvent::Progress { scanned, total: scanned });

    // Pre-extract cached hashes to avoid borrowing store inside par_iter
    let cached: std::collections::HashMap<String, String> = need_hash.iter()
        .filter_map(|(rel, _, size)| {
            store.state.files.get(rel)
                .filter(|rec| rec.size == *size)
                .map(|rec| (rel.clone(), rec.hash.clone()))
        })
        .collect();

    // Parallel hash of mtime-changed candidates
    let mut to_copy: Vec<(String, PathBuf, u64, String)> = need_hash
        .into_par_iter()
        .filter_map(|(rel, abs, size)| {
            let hash = hash_file(&abs).ok()?;
            if let Some(cached_hash) = cached.get(&rel) {
                if *cached_hash == hash { return None; }
            }
            Some((rel, abs, size, hash))
        })
        .collect();

    // Parallel hash of size-changed files
    let size_changed_hashed: Vec<(String, PathBuf, u64, String)> = size_changed
        .into_par_iter()
        .filter_map(|(rel, abs, size)| {
            let hash = hash_file(&abs).ok()?;
            Some((rel, abs, size, hash))
        })
        .collect();

    to_copy.extend(size_changed_hashed);
    (to_copy, seen)
}

pub fn full_sync(
    src: &Path,
    dst: &Path,
    store: &mut Store,
    delete_removed: bool,
    excludes: &[String],
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) {
    let (to_copy, seen) = scan_needed(src, store, excludes, tx);

    for (rel, abs, size, hash) in to_copy {
        let dst_path = dst.join(&rel);
        atomic_copy(&abs, &dst_path, &rel, hash, size, store, tx);
    }

    if delete_removed {
        let removed: Vec<String> = store.state.files.keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        for rel in removed {
            let dst_path = dst.join(&rel);
            let _ = std::fs::remove_file(&dst_path);
            store.state.files.remove(&rel);
            store.mark_dirty();
            let _ = tx.send(SyncEvent::Deleted { rel });
        }
    }

    store.state.last_sync = Some(Local::now());
    store.mark_dirty();
}

fn atomic_copy(
    src: &Path, dst: &Path, rel: &str,
    hash: String, size: u64,
    store: &mut Store,
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) {
    if let Some(parent) = dst.parent() { let _ = std::fs::create_dir_all(parent); }
    let tmp = dst.with_file_name(format!(
        "{}.svtmp", dst.file_name().unwrap_or_default().to_string_lossy()
    ));
    match std::fs::copy(src, &tmp) {
        Ok(bytes) => {
            if let Err(e) = std::fs::rename(&tmp, dst) {
                let _ = std::fs::remove_file(&tmp);
                let _ = tx.send(SyncEvent::Error { rel: rel.to_string(), err: e.to_string() });
                return;
            }
            store.state.files.insert(rel.to_string(), FileRecord {
                hash, size, modified: Local::now(),
            });
            store.state.total_synced += 1;
            store.state.total_bytes += bytes;
            store.mark_dirty();
            let _ = tx.send(SyncEvent::Copied { rel: rel.to_string(), bytes });
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            let _ = tx.send(SyncEvent::Error { rel: rel.to_string(), err: e.to_string() });
        }
    }
}

pub fn sync_file(
    abs: &Path,
    src: &Path,
    dst: &Path,
    store: &mut Store,
    excludes: &ExcludeSet,
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) {
    let rel_path = match abs.strip_prefix(src) { Ok(r) => r, Err(_) => return };
    let rel = nfc_path(rel_path.to_string_lossy()).replace('\\', "/");
    if excludes.matches(&rel) { return; }

    let dst_path = dst.join(rel_path);

    if !abs.exists() {
        let _ = std::fs::remove_file(&dst_path);
        store.state.files.remove(&rel);
        store.mark_dirty();
        let _ = tx.send(SyncEvent::Deleted { rel });
        return;
    }

    let meta = match std::fs::metadata(abs) {
        Ok(m) => m,
        Err(e) => { let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() }); return; }
    };
    let size = meta.len();

    if let Some(rec) = store.state.files.get(&rel) {
        if rec.size == size {
            // mtime check before hashing
            let mtime = meta.modified()
                .ok()
                .map(|t| chrono::DateTime::<Local>::from(t).timestamp())
                .unwrap_or(0);
            if rec.modified.timestamp() == mtime { return; }

            let hash = match hash_file(abs) {
                Ok(h) => h,
                Err(e) => { let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() }); return; }
            };
            if rec.hash == hash { return; }
            atomic_copy(abs, &dst_path, &rel, hash, size, store, tx);
            return;
        }
    }

    let hash = match hash_file(abs) {
        Ok(h) => h,
        Err(e) => { let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() }); return; }
    };
    atomic_copy(abs, &dst_path, &rel, hash, size, store, tx);
}

pub fn fmt_bytes(b: u64) -> String {
    if b < 1024 { format!("{} B", b) }
    else if b < 1024 * 1024 { format!("{:.1} KB", b as f64 / 1024.0) }
    else if b < 1024 * 1024 * 1024 { format!("{:.1} MB", b as f64 / 1024.0 / 1024.0) }
    else { format!("{:.2} GB", b as f64 / 1024.0 / 1024.0 / 1024.0) }
}
