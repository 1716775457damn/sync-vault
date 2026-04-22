use crate::state::{ExcludeSet, FileRecord, Store};
use anyhow::Result;
use chrono::Local;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Normalise a path string to NFC on macOS (HFS+ returns NFD).
/// On other platforms returns the string unchanged.
#[inline]
fn nfc_path(s: std::borrow::Cow<'_, str>) -> String {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        OsStr::from_bytes(s.as_bytes()).to_string_lossy().into_owned()
    }
    #[cfg(not(target_os = "macos"))]
    s.into_owned()
}

pub enum SyncEvent {
    Copied { rel: String, bytes: u64 },
    Deleted { rel: String },
    Error { rel: String, err: String },
    /// scanned == total signals completion (total == 0 means unknown)
    Progress { scanned: usize, total: usize },
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024]; // 256 KB — matches SSD optimal I/O size
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Phase 1: scan src without holding the store lock.
/// Returns list of (rel, abs, size, hash) that need copying.
fn scan_needed(
    src: &Path,
    store: &Store,
    excludes: &[String],
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) -> (Vec<(String, PathBuf, u64, String)>, std::collections::HashSet<String>) {
    let ex = ExcludeSet::new(excludes); // compile once, reuse per file
    let mut to_copy: Vec<(String, PathBuf, u64, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut scanned = 0usize;

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

        let size = match std::fs::metadata(abs) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        let existing = store.state.files.get(&rel);
        let (needs_copy, cached_hash) = match existing {
            None => (true, None),
            Some(rec) => {
                if rec.size != size {
                    (true, None)
                } else {
                    match hash_file(abs) {
                        Ok(h) => { let changed = h != rec.hash; (changed, Some(h)) }
                        Err(_) => (true, None),
                    }
                }
            }
        };

        if !needs_copy { continue; }

        let hash = match cached_hash {
            Some(h) => h,
            None => match hash_file(abs) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() });
                    continue;
                }
            }
        };

        to_copy.push((rel, abs.to_path_buf(), size, hash));
    }

    let _ = tx.send(SyncEvent::Progress { scanned, total: scanned });
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
    // Phase 1: scan without lock (store is already locked by caller, but
    // we do the expensive I/O before touching state)
    let (to_copy, seen) = scan_needed(src, store, excludes, tx);

    // Phase 2: copy files and update state
    for (rel, abs, size, hash) in to_copy {
        let dst_path = dst.join(&rel);
        atomic_copy(&abs, &dst_path, &rel, hash, size, store, tx);
    }

    // Phase 3: delete removed
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

/// Atomic copy: write to a temp file first, then rename — prevents corrupt dst on crash
fn atomic_copy(
    src: &Path,
    dst: &Path,
    rel: &str,
    hash: String,
    size: u64,
    store: &mut Store,
    tx: &std::sync::mpsc::Sender<SyncEvent>,
) {
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Append .svtmp to full filename (not replace extension) to avoid name collisions
    let tmp = dst.with_file_name(format!(
        "{}.svtmp",
        dst.file_name().unwrap_or_default().to_string_lossy()
    ));
    match std::fs::copy(src, &tmp) {
        Ok(bytes) => {
            // Atomic rename
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
    let rel_path = match abs.strip_prefix(src) {
        Ok(r) => r,
        Err(_) => return,
    };
    let rel = rel_path.to_string_lossy();
    let rel = nfc_path(rel).replace('\\', "/");

    if excludes.matches(&rel) { return; }

    // Build dst_path directly from rel_path — no second strip_prefix needed
    let dst_path = dst.join(rel_path);

    if !abs.exists() {
        let _ = std::fs::remove_file(&dst_path);
        store.state.files.remove(&rel);
        store.mark_dirty();
        let _ = tx.send(SyncEvent::Deleted { rel });
        return;
    }

    let size = match std::fs::metadata(abs) {
        Ok(m) => m.len(),
        Err(e) => { let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() }); return; }
    };

    // Quick size check before hashing
    if let Some(rec) = store.state.files.get(&rel) {
        if rec.size == size {
            let hash = match hash_file(abs) {
                Ok(h) => h,
                Err(e) => { let _ = tx.send(SyncEvent::Error { rel, err: e.to_string() }); return; }
            };
            if rec.hash == hash { return; }
            atomic_copy(abs, &dst_path, &rel, hash, size, store, tx);
            return;
        }
    }

    // Size changed or new file
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
