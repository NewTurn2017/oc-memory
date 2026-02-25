use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// File system observer for automatic memory ingestion
pub struct FileObserver {
    watch_dirs: Vec<PathBuf>,
    extensions: Vec<String>,
    recursive: bool,
}

/// Event emitted when a relevant file changes
#[derive(Debug, Clone)]
pub struct FileEvent {
    pub path: PathBuf,
    pub event_type: FileEventType,
}

#[derive(Debug, Clone)]
pub enum FileEventType {
    Created,
    Modified,
}

impl FileObserver {
    pub fn new(watch_dirs: Vec<PathBuf>, extensions: Vec<String>, recursive: bool) -> Self {
        Self {
            watch_dirs,
            extensions,
            recursive,
        }
    }

    /// Start watching and return a channel of file events
    pub async fn watch(&self) -> Result<mpsc::Receiver<FileEvent>> {
        let (tx, rx) = mpsc::channel(100);
        let extensions = self.extensions.clone();

        let (notify_tx, mut notify_rx) = mpsc::channel(100);

        let mut watcher = RecommendedWatcher::new(
            move |res: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = notify_tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        )?;

        let mode = if self.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        for dir in &self.watch_dirs {
            if dir.exists() {
                watcher.watch(dir, mode)?;
                tracing::info!(dir = %dir.display(), "Watching directory");
            } else {
                tracing::warn!(dir = %dir.display(), "Watch directory does not exist, skipping");
            }
        }

        // Spawn event processing task
        tokio::spawn(async move {
            let _watcher = watcher; // Keep watcher alive

            while let Some(event) = notify_rx.recv().await {
                let event_type = match event.kind {
                    EventKind::Create(_) => Some(FileEventType::Created),
                    EventKind::Modify(_) => Some(FileEventType::Modified),
                    _ => None,
                };

                if let Some(event_type) = event_type {
                    for path in event.paths {
                        if is_relevant_file(&path, &extensions) {
                            let file_event = FileEvent {
                                path,
                                event_type: event_type.clone(),
                            };
                            if tx.send(file_event).await.is_err() {
                                return; // Receiver dropped
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

fn is_relevant_file(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|e| e == ext))
        .unwrap_or(false)
}
