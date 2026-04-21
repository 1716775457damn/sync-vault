<div align="center">

# 🔄 sync-vault

**增量文件同步与备份工具 — 实时监听，只同步变更，原生 GUI**

[![Release](https://img.shields.io/github/v/release/1716775457damn/sync-vault?style=flat-square&color=28a745)](https://github.com/1716775457damn/sync-vault/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/1716775457damn/sync-vault/release.yml?style=flat-square&label=CI)](https://github.com/1716775457damn/sync-vault/actions)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?style=flat-square)](https://www.rust-lang.org)

单一二进制，无需安装。SHA-256 增量校验，只复制真正变更的文件；原子写入，崩溃不损坏目标文件；300ms 防抖实时监听，支持深色/浅色主题。

[下载](#-下载) · [功能](#-功能) · [使用方式](#-使用方式) · [架构](#-架构) · [本地构建](#-本地构建)

</div>

---

## 📦 下载

前往 [Releases](https://github.com/1716775457damn/sync-vault/releases) 下载最新版本：

| 平台 | 文件 | 说明 |
|------|------|------|
| Windows | `sync-vault-windows-x86_64.exe` | 双击即用，无需安装 |
| macOS (Apple Silicon) | `sync-vault-macos-aarch64.tar.gz` | M 系列芯片 |
| macOS (Intel) | `sync-vault-macos-x86_64.tar.gz` | x86_64 |
| Linux | `sync-vault-linux-x86_64.tar.gz` | x86_64 |

> **macOS 首次打开提示"未验证的开发者"**：系统设置 → 隐私与安全性 → 仍然打开

---

## ✨ 功能

### 同步引擎

- **两阶段增量检测**：先比对文件大小（O(1) 快速跳过），大小相同再计算 SHA-256 哈希确认，最大化跳过未变更文件
- **原子写入**：先写 `<file>.svtmp` 临时文件，再 `rename()` 到目标路径，崩溃或断电不会产生损坏的目标文件
- **同步删除**（可选）：源目录删除的文件，目标目录同步删除
- **手动全量同步**：运行中随时触发一次完整增量扫描，无需重启
- **安全守卫**：
  - 源 = 目标时拒绝启动，防止无限循环
  - 目标是源的子目录时拒绝启动，防止递归复制

### 实时监听

- **OS 级文件系统事件**：通过 `notify` crate 监听，延迟 < 100ms
- **300ms 防抖**：IDE 自动保存、构建输出等连续写入合并为一次同步
- **⏸ 暂停/恢复**：临时停止响应文件变动，不停止已运行的同步任务
- **独立防抖线程**：计时器在独立线程运行，不阻塞 UI，不丢失最后一次事件

### 排除规则

- **默认规则**：`.git`、`.svn`、`node_modules`、`__pycache__`、`target`、`*.tmp`、`*.swp`
- **自定义规则**：支持精确名称（`node_modules`）和通配符（`*.log`）
- **深度匹配**：路径任意层级中出现的 `node_modules` 均被排除，不只匹配根目录
- **持久化**：排除规则跨重启保存

### 界面

- **深色/浅色主题**：点击 ☀️/🌙 切换，或按 `T` 键
- **实时日志**：每条同步操作带时间戳，颜色区分类型（绿色=复制、橙色=删除、红色=错误）
- **日志过滤**：关键词过滤，一键切换「只看错误」
- **本次统计**：当前会话复制的文件数和字节数
- **累计统计**：历史所有会话的总文件数和总字节数
- **扫描进度**：全量扫描时显示已扫描文件数
- **路径校验**：无效路径红色高亮并显示错误信息
- **配置持久化**：源/目标路径和所有设置跨重启恢复

---

## 🚀 使用方式

1. 打开 sync-vault
2. 填写「源目录」（要监控的文件夹）和「目标目录」（备份到哪里）
3. 按需勾选「同步删除」和配置排除规则
4. 点击 **▶ 开始同步**

启动后会先执行一次全量增量扫描，然后持续监听文件变动，自动同步。

```
┌──────────────────────────────────────────────────────────┐
│ 源目录:   [/Users/me/project        ] 📁                  │
│ 目标目录: [/Volumes/Backup/project  ] 📁                  │
│ [✓ 同步删除] [▼ 排除规则] [☀️] [⏸ 暂停] [🔄 立即同步] [⏹ 停止] │
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

## 🏗 架构

```
src/
├── main.rs     # 入口，窗口配置，内嵌 CJK 字体（NotoSansSC）
├── app.rs      # GUI：配置面板、日志、统计、暂停/恢复
├── syncer.rs   # 核心引擎：SHA-256 哈希、原子复制、全量/增量同步
├── watcher.rs  # 文件系统事件监听，300ms 防抖
├── state.rs    # 哈希快照持久化、配置、ExcludeSet 匹配
└── theme.rs    # 深色/浅色主题 visuals
```

| 组件 | Crate | 说明 |
|------|-------|------|
| GUI | `egui` / `eframe` | 即时模式，原生渲染，无 Electron |
| 文件监听 | `notify` | 跨平台 OS 级事件 |
| 哈希 | `sha2` | SHA-256，工业标准 |
| 目录遍历 | `walkdir` | 递归遍历 |
| 序列化 | `serde_json` | 哈希快照和配置持久化 |
| 文件对话框 | `rfd` | 原生文件选择器 |

---

## ⚡ 性能

- **大小优先**：文件大小不同直接跳过哈希计算
- **哈希复用**：大小相同时，哈希计算一次同时用于比较和状态更新
- **流式遍历**：目录条目逐个处理，无需预先分配内存
- **ExcludeSet 预编译**：每次扫描编译一次，每个路径段 O(1) HashSet 查找
- **防抖合并**：300ms 窗口内的连续变更合并为一次同步，避免冗余 I/O
- **状态写入节流**：哈希快照最多每 3 秒写一次磁盘
- **日志过滤缓存**：过滤索引缓存，仅在日志或过滤词变化时重建

---

## 🔧 本地构建

需要 [Rust](https://rustup.rs/) stable 工具链。

```bash
git clone https://github.com/1716775457damn/sync-vault.git
cd sync-vault
cargo build --release

# 产物
./target/release/sync-vault        # macOS / Linux
./target/release/sync-vault.exe    # Windows
```

Linux 额外依赖：

```bash
sudo apt-get install -y \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev libgtk-3-dev
```

---

## 🤖 CI/CD

推送 `v*` tag 触发四平台并行构建：

```bash
git tag v4.5.0
git push origin v4.5.0
```

GitHub Actions 在 Windows / macOS ARM / macOS Intel / Ubuntu 上并行构建，约 5 分钟后在 [Releases](https://github.com/1716775457damn/sync-vault/releases) 生成全部二进制。

---

## 📄 License

MIT © 2025
