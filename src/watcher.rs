use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 300;

pub fn start(src: PathBuf, tx: Sender<Vec<PathBuf>>) -> anyhow::Result<RecommendedWatcher> {
    let pending: Arc<Mutex<HashMap<PathBuf, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let pending_flush = pending.clone();
    let tx_flush = tx.clone();

    // stop_tx is held by the watcher closure; when the watcher is dropped,
    // stop_tx drops too, causing stop_rx.recv_timeout to return Disconnected → thread exits.
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    // Flush thread: uses recv_timeout instead of sleep+try_recv — zero CPU when idle.
    std::thread::spawn(move || {
        loop {
            match stop_rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let mut map = pending_flush.lock().unwrap();
            let ready: Vec<PathBuf> = map
                .iter()
                .filter(|(_, t)| t.elapsed() >= Duration::from_millis(DEBOUNCE_MS))
                .map(|(p, _)| p.clone())
                .collect();
            if !ready.is_empty() {
                for p in &ready { map.remove(p); }
                drop(map);
                if tx_flush.send(ready).is_err() { return; }
            }
        }
    });

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let _keep = &stop_tx; // keep stop_tx alive until watcher is dropped
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
