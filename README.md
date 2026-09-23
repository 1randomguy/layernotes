# layernotes

Bottom-layer Wayland post-it notes, stored and edited as markdown files.

`layernotes` renders one fullscreen surface **per monitor** on the layer-shell
`Bottom` layer, so notes sit **behind normal windows but in front of the
wallpaper**. Notes are ordinary `.md` files with a YAML frontmatter block for
their position, size and appearance.

It is built on the same stack as [ashell](https://github.com/MalpenZibo/ashell):
Rust (edition 2024), the [iced_layershell](https://github.com/MalpenZibo/iced_layershell)
backend for iced 0.14, and the Elm architecture.

## Features

- One layer-shell surface per output on the `Bottom` layer (configurable layer).
- Markdown notes rendered with iced's built-in markdown widget.
- Double-click a note (or its header button) to edit the raw markdown in a
  syntax-highlighted `text_editor`; toggle back for the rendered preview.
- Drag notes by their header; resize from the bottom-right handle. Position and
  size are persisted to the file's frontmatter, and notes can be dragged from
  one monitor to another.
- Create notes by double-clicking empty desktop.
- Delete notes with a two-step confirm button.
- Notes are watched with `inotify`, so edits made in another editor are picked
  up live. The config file is hot-reloaded too.
- Autosave is debounced; geometry changes save on release.
- Theming via the built-in [Catppuccin](https://catppuccin.com) palettes
  (Mocha by default), with Macchiato, Frappe, Latte, Dark and Light available.
  Notes use the palette's dark `base` with `text` and a `mauve` focus ring; the
  selected theme drives the header, borders, selection ring and editor colours.

## Note format

Each note is a markdown file in the notes directory:

```markdown
---
id: 0f3c1e2a-6b1d-4c2e-9a3b-1f2e3d4c5b6a
title: Grocery
x: 120.0
y: 80.0
width: 260.0
height: 220.0
color: "#f9e2af"
font_size: 15.0
output: DP-1
created: 2026-09-23T10:00:00+02:00
updated: 2026-09-23T10:05:00+02:00
---

# Grocery
- milk
- eggs
```

Only `id`, `x`, `y`, `width`, `height`, `created` and `updated` are required;
everything else is optional. Files without frontmatter are loaded as body-only
markdown and get frontmatter on the next save.

## Configuration

On first run a config file is written to `~/.config/layernotes/config.toml`:

```toml
log_level = "warn"
notes_dir = "~/.local/share/layernotes/notes"
outputs = "All"          # "All", "Active", or a list like ["DP-1", "HDMI-A-1"]
layer = "Bottom"         # "Background", "Bottom", "Top", "Overlay"
font_size = 15.0
scale_factor = 1.0
default_width = 260.0
default_height = 220.0
# default_color = "#f9e2af"  # optional; defaults to the theme's base
autosave_debounce_ms = 500
theme = "CatppuccinMocha" # CatppuccinMocha | CatppuccinMacchiato | CatppuccinFrappe | CatppuccinLatte | Dark | Light

# Multipliers applied to the note font size when rendering markdown.
[markdown]
h1_scale = 1.5
h2_scale = 1.35
h3_scale = 1.2
h4_scale = 1.1
h5_scale = 1.0
h6_scale = 0.95
code_scale = 0.8
spacing_scale = 0.75
```

Per-note appearance can still be overridden in the note's own frontmatter via
`color` and `font_size`.

A `--config-path` flag overrides the config location.

## Build & run

With Nix flakes (provides the Rust toolchain and all system dependencies):

```bash
direnv allow          # or: nix develop
make build
make start
make check            # fmt + cargo check + clippy -D warnings
make test
```

Or, in an environment that already has the system dependencies
(`libxkbcommon`, `libwayland`, `vulkan-loader`, `libGL`, `pkg-config`):

```bash
cargo build --release
./target/release/layernotes
```

## Usage

- **New note**: double-click empty desktop.
- **Edit**: double-click a note, or click `edit` in its header. Click `done`,
  press `Esc`, click the desktop, or move onto another window / the bar to go
  back to the preview.
- **Move**: drag the note's header, including across monitors (the note is
  re-homed to the monitor it is dropped on).
- **Resize**: drag the bottom-right corner.
- **Delete**: click `x`, then `sure?` to confirm.

## Project layout

```
src/
├── main.rs      # CLI, logging, config load, layer-shell application setup
├── app.rs       # App state, Message, update, view, subscriptions
├── config.rs    # TOML config + hot reload
├── note.rs      # Note model + YAML frontmatter (de)serialization
├── store.rs     # load/save + notes directory watcher
├── outputs.rs   # one fullscreen layer surface per monitor
├── theme.rs     # palette helpers
└── widgets/
    └── note_card.rs  # note rendering, editing, drag/resize handles
```

## Known limitations

- A fullscreen bottom surface captures clicks on empty desktop (expected on
  compositors without desktop icons).
- The keyboard is requested on demand; if a compositor refuses keyboard focus to
  bottom-layer surfaces, editing will not receive input.
- Unfocus is driven by the pointer leaving the surface. Moving the pointer onto
  another window or the bar ends editing, which is also how a click on them is
  detected (the backend exposes no keyboard-focus events).
- Per-note `color` is honoured when rendering but there is no colour picker yet.
