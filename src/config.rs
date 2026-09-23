use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use inotify::{Inotify, WatchMask};
use serde::{Deserialize, Serialize};

use iced::Subscription;
use iced::futures::{SinkExt, StreamExt};
use iced::stream;

use crate::xdg;

pub const DEFAULT_CONFIG_FILE_PATH: &str = "~/.config/layernotes/config.toml";

/// How long to wait for the config file to stop changing before reloading it.
/// Coalesces the delete/rename burst of an atomic save into a single reload.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub log_level: String,
    /// Directory holding the markdown note files.
    pub notes_dir: String,
    /// Which monitors to render notes on.
    pub outputs: Outputs,
    /// Layer-shell layer to render on. `Bottom` keeps notes behind windows.
    pub layer: Layer,
    pub font_size: f32,
    pub scale_factor: f64,
    pub default_width: f32,
    pub default_height: f32,
    /// Optional fixed background for new notes. When unset, the active theme's
    /// yellow is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_color: Option<String>,
    pub autosave_debounce_ms: u64,
    /// Colour theme used for the note chrome (buttons, borders, text).
    pub theme: Theme,
    /// Markdown rendering tweaks.
    pub markdown: MarkdownConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_level: "warn".to_owned(),
            notes_dir: default_notes_dir().to_string_lossy().into_owned(),
            outputs: Outputs::default(),
            layer: Layer::default(),
            font_size: 15.0,
            scale_factor: 1.0,
            default_width: 260.0,
            default_height: 220.0,
            default_color: None,
            autosave_debounce_ms: 500,
            theme: Theme::default(),
            markdown: MarkdownConfig::default(),
        }
    }
}

impl Config {
    /// Fully expanded path to the notes directory.
    pub fn notes_path(&self) -> PathBuf {
        xdg::expand_tilde(&self.notes_dir)
    }

    pub fn autosave_debounce(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.autosave_debounce_ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Outputs {
    Mode(OutputsMode),
    Targets(Vec<String>),
}

impl Default for Outputs {
    fn default() -> Self {
        Self::Mode(OutputsMode::All)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputsMode {
    All,
    Active,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum Layer {
    Background,
    #[default]
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    CatppuccinMocha,
    CatppuccinMacchiato,
    CatppuccinFrappe,
    CatppuccinLatte,
    Dark,
    Light,
}

impl Theme {
    /// The corresponding iced theme, used both for the app and to derive the
    /// note chrome palette.
    pub fn iced(self) -> iced::Theme {
        match self {
            Theme::CatppuccinMocha => iced::Theme::CatppuccinMocha,
            Theme::CatppuccinMacchiato => iced::Theme::CatppuccinMacchiato,
            Theme::CatppuccinFrappe => iced::Theme::CatppuccinFrappe,
            Theme::CatppuccinLatte => iced::Theme::CatppuccinLatte,
            Theme::Dark => iced::Theme::Dark,
            Theme::Light => iced::Theme::Light,
        }
    }
}

/// Multipliers applied to the note font size when rendering markdown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkdownConfig {
    pub h1_scale: f32,
    pub h2_scale: f32,
    pub h3_scale: f32,
    pub h4_scale: f32,
    pub h5_scale: f32,
    pub h6_scale: f32,
    pub code_scale: f32,
    pub spacing_scale: f32,
}

impl Default for MarkdownConfig {
    fn default() -> Self {
        Self {
            h1_scale: 1.5,
            h2_scale: 1.35,
            h3_scale: 1.2,
            h4_scale: 1.1,
            h5_scale: 1.0,
            h6_scale: 0.95,
            code_scale: 0.8,
            spacing_scale: 0.75,
        }
    }
}

pub fn default_notes_dir() -> PathBuf {
    xdg::data_dir().join("layernotes").join("notes")
}

pub fn default_config_path() -> PathBuf {
    xdg::expand_tilde(DEFAULT_CONFIG_FILE_PATH)
}

/// Read the config from `path`, creating a default file on first run.
pub fn get_config(path: Option<PathBuf>) -> Result<(Config, PathBuf)> {
    let path = path.unwrap_or_else(default_config_path);

    if path.exists() {
        let config = read_config(&path)?;
        Ok((config, path))
    } else {
        let config = Config::default();
        if let Err(e) = write_default_config(&path, &config) {
            log::warn!("Could not write default config to {}: {e}", path.display());
        }
        Ok((config, path))
    }
}

pub fn read_config(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))
}

fn write_default_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(config).context("serialize default config")?;
    std::fs::write(path, text).with_context(|| format!("write config {}", path.display()))
}

/// Whether `name` (a Wayland output name) should be rendered on.
pub fn output_matches(outputs: &Outputs, name: &str) -> bool {
    match outputs {
        Outputs::Mode(OutputsMode::All | OutputsMode::Active) => true,
        Outputs::Targets(targets) => targets.iter().any(|target| name.contains(target.as_str())),
    }
}

/// Watch the config file for changes and emit whenever it is written.
pub fn subscription(path: &Path) -> Subscription<()> {
    let path = path.to_path_buf();

    Subscription::run_with(path, |path| {
        let path = path.clone();
        stream::channel(16, async move |mut output| {
            let Some(parent) = path.parent().map(Path::to_path_buf) else {
                return;
            };
            let Some(file_name) = path.file_name().map(|name| name.to_os_string()) else {
                return;
            };

            let inotify = match Inotify::init() {
                Ok(inotify) => inotify,
                Err(e) => {
                    log::error!("config inotify init failed: {e}");
                    return;
                }
            };

            let mask = WatchMask::CREATE
                | WatchMask::DELETE
                | WatchMask::MODIFY
                | WatchMask::MOVED_TO
                | WatchMask::MOVED_FROM;

            if let Err(e) = inotify.watches().add(parent.as_path(), mask) {
                log::error!("Failed to watch {}: {e}", parent.display());
                return;
            }

            let buffer = [0u8; 4096];
            let mut events = match inotify.into_event_stream(buffer) {
                Ok(events) => events.ready_chunks(16),
                Err(e) => {
                    log::error!("config inotify stream failed: {e}");
                    return;
                }
            };

            while let Some(chunk) = events.next().await {
                let relevant = chunk.iter().flatten().any(|event| {
                    event
                        .name
                        .as_ref()
                        .is_some_and(|name| name.to_string_lossy() == file_name.to_string_lossy())
                });

                if !relevant {
                    continue;
                }

                // Wait until the file stops changing so we read a complete
                // write instead of racing an editor's atomic save.
                while let Ok(Some(_)) = tokio::time::timeout(WATCH_DEBOUNCE, events.next()).await {}

                if output.send(()).await.is_err() {
                    break;
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
log_level = "warn"
notes_dir = "/home/bene/.local/share/layernotes/notes"
outputs = "All"
layer = "Bottom"
font_size = 15.0
scale_factor = 1.0
default_width = 260.0
default_height = 220.0
autosave_debounce_ms = 500
theme = "CatppuccinMocha"

[markdown]
h1_scale = 1.3
h2_scale = 1.2
h3_scale = 1.1
h4_scale = 1.0
h5_scale = 1.0
h6_scale = 1.0
code_scale = 0.8
spacing_scale = 0.65
"#;

    #[test]
    fn parses_sample_config() {
        let config: Config = toml::from_str(SAMPLE).expect("sample config should parse");
        assert_eq!(config.theme, Theme::CatppuccinMocha);
        assert_eq!(config.layer, Layer::Bottom);
        assert_eq!(config.outputs, Outputs::Mode(OutputsMode::All));
        assert_eq!(config.markdown.h1_scale, 1.3);
        assert_eq!(config.markdown.spacing_scale, 0.65);
        assert!(config.default_color.is_none());
    }
}
