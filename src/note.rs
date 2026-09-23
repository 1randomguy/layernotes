use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use iced::widget::{markdown, text_editor};

use crate::config::Config;

/// Metadata stored in the YAML frontmatter of a note file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteMeta {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub created: DateTime<Local>,
    pub updated: DateTime<Local>,
}

impl Default for NoteMeta {
    fn default() -> Self {
        let now = Local::now();
        Self {
            id: Uuid::new_v4(),
            title: None,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            color: None,
            font_size: None,
            output: None,
            created: now,
            updated: now,
        }
    }
}

/// A loaded note: its metadata, markdown body and runtime editing state.
pub struct Note {
    pub meta: NoteMeta,
    pub body: String,
    pub path: PathBuf,
    /// Parsed markdown items, cached for rendering.
    pub items: Vec<markdown::Item>,
    /// Editor buffer, present only while the note is being edited.
    pub editor: Option<text_editor::Content>,
    pub editing: bool,
    pub dirty: bool,
    /// Two-step delete confirmation.
    pub confirm_delete: bool,
}

impl Note {
    pub fn new(meta: NoteMeta, body: String, path: PathBuf) -> Self {
        let mut note = Self {
            meta,
            body,
            path,
            items: Vec::new(),
            editor: None,
            editing: false,
            dirty: false,
            confirm_delete: false,
        };
        note.refresh_items();
        note
    }

    /// Build a brand new note for the given directory.
    pub fn create(dir: &Path, config: &Config, x: f32, y: f32, output: Option<String>) -> Self {
        let meta = NoteMeta {
            x,
            y,
            width: config.default_width,
            height: config.default_height,
            color: config.default_color.clone(),
            output,
            ..NoteMeta::default()
        };
        let path = dir.join(file_name(&meta));
        Self::new(meta, String::new(), path)
    }

    /// Load a note from disk.
    pub fn load(path: &Path, config: &Config) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let (mut meta, body) = parse_note(&text);
        if meta.width <= 0.0 {
            meta.width = config.default_width;
        }
        if meta.height <= 0.0 {
            meta.height = config.default_height;
        }
        if meta.color.is_none() {
            meta.color = config.default_color.clone();
        }
        Ok(Self::new(meta, body, path.to_path_buf()))
    }

    /// The exact bytes that should be written to disk.
    pub fn to_file_string(&self) -> String {
        serialize_note(&self.meta, &self.body)
    }

    /// Re-parse the body into renderable markdown items.
    pub fn refresh_items(&mut self) {
        self.items = markdown::parse(&self.body).collect();
    }

    /// Ensure an editor buffer exists, seeded from the current body with the
    /// cursor placed at the end of the note.
    pub fn editor_mut(&mut self) -> &mut text_editor::Content {
        if self.editor.is_none() {
            let mut content = text_editor::Content::with_text(&self.body);
            content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
            self.editor = Some(content);
        }
        self.editor.as_mut().expect("editor just created")
    }

    /// A short human readable title for the note header.
    pub fn display_title(&self) -> String {
        if let Some(title) = &self.meta.title
            && !title.is_empty()
        {
            return title.clone();
        }
        self.body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| {
                line.trim_start_matches('#')
                    .trim()
                    .chars()
                    .take(40)
                    .collect::<String>()
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "note".to_owned())
    }
}

/// Split a note file into its frontmatter metadata and markdown body.
///
/// Files without frontmatter are treated as body-only.
pub fn parse_note(text: &str) -> (NoteMeta, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or("");

    if first.trim() != "---" {
        return (NoteMeta::default(), text.to_owned());
    }

    let mut yaml = String::new();
    let mut body = String::new();
    let mut in_body = false;

    for line in lines {
        if !in_body && line.trim() == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body.push_str(line);
        } else {
            yaml.push_str(line);
        }
    }

    let meta = serde_yaml_ng::from_str::<NoteMeta>(&yaml).unwrap_or_default();
    (meta, body)
}

/// Render metadata and body back into a note file.
pub fn serialize_note(meta: &NoteMeta, body: &str) -> String {
    let yaml = serde_yaml_ng::to_string(meta).unwrap_or_default();
    let body = body.trim_start_matches('\n');
    if body.is_empty() {
        format!("---\n{yaml}---\n")
    } else {
        format!("---\n{yaml}---\n\n{body}")
    }
}

/// Stable, human friendly file name derived from the title and id.
pub fn file_name(meta: &NoteMeta) -> String {
    let base = meta
        .title
        .as_deref()
        .map(slugify)
        .filter(|slug| !slug.is_empty())
        .unwrap_or_else(|| "note".to_owned());
    let short = meta.id.simple().to_string();
    format!("{base}-{}.md", &short[..short.len().min(8)])
}

/// Lowercase, dash-separated slug limited to ASCII alphanumerics.
pub fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut pending_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    slug.truncate(48);
    slug.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  multiple   spaces  "), "multiple-spaces");
        assert_eq!(slugify("ünicode stays"), "nicode-stays");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn frontmatter_round_trip() {
        let mut meta = NoteMeta::default();
        meta.title = Some("Grocery".to_owned());
        meta.x = 12.0;
        meta.y = 34.0;
        meta.width = 200.0;
        meta.height = 150.0;
        meta.color = Some("#f9e2af".to_owned());

        let body = "# Grocery\n- milk\n- eggs\n";
        let text = serialize_note(&meta, body);
        let (parsed, parsed_body) = parse_note(&text);

        assert_eq!(parsed.id, meta.id);
        assert_eq!(parsed.title.as_deref(), Some("Grocery"));
        assert_eq!(parsed.x, 12.0);
        assert_eq!(parsed.color.as_deref(), Some("#f9e2af"));
        assert_eq!(parsed_body.trim(), body.trim());
    }

    #[test]
    fn plain_markdown_is_body_only() {
        let (meta, body) = parse_note("# Just markdown\n");
        assert!(meta.title.is_none());
        assert_eq!(body, "# Just markdown\n");
    }

    #[test]
    fn empty_body_round_trip() {
        let meta = NoteMeta::default();
        let text = serialize_note(&meta, "");
        let (parsed, body) = parse_note(&text);
        assert_eq!(parsed.id, meta.id);
        assert!(body.trim().is_empty());
    }
}
