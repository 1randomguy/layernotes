use std::path::{Path, PathBuf};

use iced::Subscription;
use iced::futures::StreamExt;
use iced::stream;
use inotify::{Inotify, WatchMask};

use crate::config::Config;
use crate::note::Note;

pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Load every `*.md` file in `dir`, sorted by creation time.
pub fn load_notes(dir: &Path, config: &Config) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return notes;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        match Note::load(&path, config) {
            Ok(note) => notes.push(note),
            Err(e) => log::warn!("Failed to load note {}: {e}", path.display()),
        }
    }

    notes.sort_by(|a, b| {
        a.meta
            .created
            .cmp(&b.meta.created)
            .then_with(|| a.meta.id.cmp(&b.meta.id))
    });
    notes
}

pub async fn write_note(path: PathBuf, content: String) -> Result<(), String> {
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))
}

pub async fn remove_note(path: PathBuf) -> Result<(), String> {
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", path.display())),
    }
}

/// Watch the notes directory for external changes.
pub fn subscription(dir: PathBuf) -> Subscription<()> {
    Subscription::run_with(dir, |dir| {
        let dir = dir.clone();
        stream::channel(16, async move |mut output| {
            let _ = std::fs::create_dir_all(&dir);

            let inotify = match Inotify::init() {
                Ok(inotify) => inotify,
                Err(e) => {
                    log::error!("inotify init failed: {e}");
                    return;
                }
            };

            let mask = WatchMask::CREATE
                | WatchMask::DELETE
                | WatchMask::MODIFY
                | WatchMask::MOVED_TO
                | WatchMask::MOVED_FROM;

            if let Err(e) = inotify.watches().add(dir.as_path(), mask) {
                log::error!("Failed to watch {}: {e}", dir.display());
                return;
            }

            let buffer = [0u8; 4096];
            let mut events = match inotify.into_event_stream(buffer) {
                Ok(events) => events.ready_chunks(16),
                Err(e) => {
                    log::error!("inotify event stream failed: {e}");
                    return;
                }
            };

            while let Some(chunk) = events.next().await {
                let relevant = chunk.iter().flatten().any(|event| {
                    event
                        .name
                        .as_ref()
                        .map_or(true, |name| name.to_string_lossy().ends_with(".md"))
                });

                if relevant && output.send(()).await.is_err() {
                    break;
                }
            }
        })
    })
}
