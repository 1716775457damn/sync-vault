use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 300;

pub fn start(src: PathBuf, tx: Sender<Vec<PathBuf>>) -> anyhow::Result<RecommendedWatcher> {
    let pending: Arc<Mutex<HashMap<PathBuf, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let pending_flush = pending.clone();
    let tx_flush = tx.clone();

    // Flush thread: exits automatically when tx_flush.send fails (receiver dropped on stop)
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let mut map = pending_flush.lock().unwrap();
            let ready: Vec<PathBuf> = map
                .iter()
                .filter(|(_, t)| t.elapsed() >= Duration::from_millis(DEBOUNCE_MS))
                .map(|(p, _)| p.clone())
                .collect();
            if !ready.is_empty() {
                for p in &ready { map.remove(p); }
                drop(map);
                if tx_flush.send(ready).is_err() { return; } // receiver gone → stop
            }
        }
    });

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let is_relevant = matches!(event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                if !is_relevant { return; }
                let now = Instant::now();
                let mut map = pending.lock().unwrap();
                for path in event.paths {
                    map.insert(path, now);
                }
            }
        },
        Config::default(),
    )?;
    watcher.watch(&src, RecursiveMode::Recursive)?;
    Ok(watcher)
}
