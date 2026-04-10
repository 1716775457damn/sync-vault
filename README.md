# 🔄 Sync Vault

> **A blazing-fast incremental file sync and backup tool built with Rust — watches your folders and automatically backs up only what changed.**

![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![License](https://img.shields.io/badge/license-MIT-green)
![Version](https://img.shields.io/badge/version-0.1.0-brightgreen)

---

## ✨ Why Sync Vault?

Most backup tools either copy everything every time (slow) or require complex configuration. **Sync Vault** uses SHA-256 hashing to detect exactly which files changed and copies only those — making backups fast regardless of how large your directory is.

- ⚡ **Incremental sync** — SHA-256 hash comparison, only changed files are copied
- 🔍 **Real-time monitoring** — detects file changes within 300ms using OS-level events
- 🛡️ **Atomic writes** — copies to a temp file first, then renames — no corrupt backups on crash
- 🚫 **Smart exclusions** — skip `.git`, `node_modules`, `target`, `*.tmp` and more
- 🖥️ **Native GUI** — built with `egui`, 60fps, no Electron, no web bloat
- 📦 **Zero dependencies** — single binary, runs anywhere

---

## 🚀 How It Works

### On Start
1. **Full incremental scan** — walks the entire source directory
2. For each file: compares **size first** (fast), then **SHA-256 hash** (only if size matches)
3. Copies only files that are new or changed
4. Updates the hash snapshot for future comparisons

### While Running
- OS-level file system events are monitored via the `notify` crate
- Events are **debounced for 300ms** — rapid saves (e.g. IDE auto-save) are batched into one sync
- Changed files are synced immediately after the debounce window

### Safety
- All copies use **atomic write**: write to `<file>.svtmp` first, then `rename()` to destination
- `rename()` is atomic on the same filesystem — destination is never in a partial state
- Hash snapshot is saved with a 3-second debounce to avoid excessive disk I/O
- **Source = destination guard** — refuses to start if src and dst are the same path
- **Recursive copy guard** — refuses to start if dst is inside src

---

## 🚀 Features

### Sync
- **Incremental by default** — size check first, hash only when needed
- **Atomic copy** — temp file + rename, no corrupt files on crash or power loss
- **Delete sync** — optionally mirror deletions from source to destination
- **Manual resync** — trigger a full scan at any time without restarting
- **Safety guards** — prevents infinite loops from src=dst or dst-inside-src

### Monitoring
- **300ms debounce** — batches rapid file events (IDE saves, build outputs)
- **⏸ Pause / Resume** — temporarily stop responding to changes without stopping the watcher
- **Dedicated flush thread** — debounce timer runs independently, never misses the last event

### Exclusions
- **Default rules** — `.git`, `.svn`, `node_modules`, `__pycache__`, `target`, `*.tmp`, `*.swp`
- **Custom rules** — add any filename, directory name, or `*.ext` pattern
- **Deep matching** — `node_modules` anywhere in the path is excluded, not just at the root
- **Fixed-height panel** — exclusion rules panel has a fixed height, log area never jumps
- **Persistent** — exclusion rules saved across restarts

### Interface
- **Real-time log** — every sync action logged with timestamp, color-coded by type
- **Log filter** — search the log by keyword
- **❌ Errors only** — one-click toggle to show only error lines
- **Session stats** — files copied and bytes transferred in the current session
- **Cumulative stats** — total files synced and bytes transferred across all sessions
- **Scan progress** — shows files scanned during full sync
- **Config persistence** — source/destination paths and settings restored on restart
- **Path validation** — red highlight and error message for invalid paths

---

## 📸 Screenshot

```
┌──────────────────────────────────────────────────────────┐
│ 源目录:  [/Users/me/project        ] 📁                   │
│ 目标目录: [/Volumes/Backup/project  ] 📁                   │
│ [✓ 同步删除] [▼ 排除规则] [⏸ 暂停] [🔄 立即同步] [⏹ 停止]  │
├──────────────────────────────────────────────────────────┤
│ 同步日志 (47条)  过滤: [     ] ❌错误  清空               │
│ [10:23:41] ✅ 开始监控: /project → /Backup/project        │
│ [10:23:42] 📋 已同步  src/main.rs  (4.2 KB)               │
│ [10:23:42] 📋 已同步  src/app.rs   (12.1 KB)              │
│ [10:24:15] 📋 已同步  Cargo.toml   (892 B)                │
│ [10:25:03] 🗑 已删除  src/old.rs                          │
├──────────────────────────────────────────────────────────┤
│ ⟳ 已扫描 1,247 个文件  本次: 3 个文件 17.2 KB             │
│ 累计同步 142 个文件  1.8 MB  |  上次: 10/15 10:25:03      │
└──────────────────────────────────────────────────────────┘
```

---

## 📥 Download & Run

### Windows
1. Go to [Releases](../../releases)
2. Download `sync-vault.exe`
3. Double-click to open

> ✅ No .NET, no Java, no Python, no Visual C++ Redistributable required.  
> Works on Windows 10 and above.

### macOS

macOS does not allow running unsigned binaries by default. Build from source:

```bash
# 1. Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 2. Clone and build
git clone https://github.com/1716775457damn/sync-vault.git
cd sync-vault
cargo build --release

# 3. Run
./target/release/sync-vault
```

**Optional: package as a `.app` bundle**

```bash
cargo install cargo-bundle
cargo bundle --release
open "target/release/bundle/osx/Sync Vault.app"
```

> ℹ️ On first launch macOS may show a security warning.  
> Go to **System Settings → Privacy & Security** and click **Open Anyway**.

> 🌏 CJK (Chinese/Japanese/Korean) fonts are embedded in the binary — no system font installation needed.

> 💡 To sync to an external drive on macOS, use the drive's mount path, e.g. `/Volumes/MyDrive/backup`.

### Linux

```bash
git clone https://github.com/1716775457damn/sync-vault.git
cd sync-vault
cargo build --release
./target/release/sync-vault
```

---

## 🛠️ Build from Source

Requires [Rust](https://rustup.rs/) (stable toolchain).

```bash
git clone https://github.com/1716775457damn/sync-vault.git
cd sync-vault
cargo build --release
# Windows: target/release/sync-vault.exe
# macOS/Linux: target/release/sync-vault
```

---

## 🏗️ Architecture

```
src/
├── main.rs      # Entry point, window setup, embedded CJK font
├── app.rs       # GUI (egui): config panel, log, stats, pause/resume
├── syncer.rs    # Core engine: SHA-256 hashing, atomic copy, full/incremental sync
├── watcher.rs   # File system event monitoring with debounce
└── state.rs     # Hash snapshot persistence, config, ExcludeSet matching
```

| Component | Crate | Why |
|-----------|-------|-----|
| GUI | `egui` / `eframe` | Immediate-mode, native, no Electron |
| File watching | `notify` | Cross-platform OS-level events |
| Hashing | `sha2` | SHA-256, industry standard |
| Directory walk | `walkdir` | Recursive traversal |
| Serialization | `serde_json` | Hash snapshot and config |

---

## ⚡ Performance

- **Size-first comparison** — files with different sizes skip hashing entirely
- **Hash caching** — when size matches, hash is computed once and reused for both comparison and state update
- **Streaming walk** — directory entries are processed one at a time, no upfront memory allocation
- **Pre-compiled ExcludeSet** — exclude patterns compiled once per scan, O(1) HashSet lookup per segment
- **Debounced events** — 300ms window prevents redundant syncs during rapid file changes
- **Atomic state writes** — hash snapshot flushed at most once every 3 seconds
- **Cached filter indices** — log filter results cached, rebuilt only when log or filter changes

---

## 🗺️ Roadmap

- [ ] Multiple sync pairs (watch several source→destination pairs simultaneously)
- [ ] Dry-run mode (preview what would be synced without copying)
- [ ] Bandwidth throttling for large files
- [ ] Sync history / undo last sync
- [ ] System tray icon

---

## 📄 License

MIT © 2025
