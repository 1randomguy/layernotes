use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use uuid::Uuid;

use iced::event::listen_with;
use iced::widget::{Space, Stack, container, mouse_area, pin, text_editor};
use iced::{
    Anchor, Element, InputRegionRect, KeyboardInteractivity, Layer, LayerShellSettings, Length,
    OutputEvent, OutputId, Padding, Point, Size, Subscription, SurfaceId, Task, Theme, alignment,
    destroy_layer_surface, keyboard, mouse, new_layer_surface, set_input_region, window,
};

use crate::config::{self, Config};
use crate::geometry::{self, OutputRect};
use crate::note::Note;
use crate::outputs::{Outputs, SurfaceKind};
use crate::store;
use crate::theme::Palette;
use crate::widgets;

/// How long a drag may stay alive after the pointer leaves a surface before it
/// is considered dropped on a foreign surface (a window, the bar, ...). Gives
/// the pointer time to enter another layernotes surface on a different monitor.
const LEAVE_GRACE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy)]
struct Drag {
    window: window::Id,
    note: Uuid,
    start_cursor: Point,
    start_pos: Point,
    /// The button was released while the pointer was outside the surface (i.e.
    /// on another monitor); the drag is waiting to be picked up there.
    released: bool,
}

#[derive(Debug, Clone, Copy)]
struct Resize {
    window: window::Id,
    note: Uuid,
    start_cursor: Point,
    start_size: Size,
}

#[derive(Debug, Clone, Copy)]
struct PendingLeave {
    window: window::Id,
    token: u64,
}

/// A fullscreen overlay surface that catches clicks outside the note being
/// edited, so clicking a window, the bar or another monitor leaves edit mode
/// (like ashell's menus). On the edited note's monitor its input region is the
/// complement of the note rectangle, so clicks on the note fall through to the
/// editor on the Bottom-layer surface; on the other monitors it catches
/// everything.
#[derive(Debug, Clone, Copy)]
struct Catcher {
    surface_id: SurfaceId,
    output: Option<OutputId>,
}

const CATCHER_NAMESPACE: &str = "layernotes-catcher";
const BOARD_NAMESPACE: &str = "layernotes-rescue";

/// The rescue board surface: a fullscreen Overlay panel listing notes whose
/// monitor is disconnected. Created on demand and destroyed when closed.
#[derive(Debug, Clone)]
struct BoardSurface {
    surface_id: SurfaceId,
    /// Output the board is shown on; notes are moved here.
    output_name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    OutputEvent(OutputEvent),
    ConfigChanged,
    NotesChanged,
    ToggleLayer,
    AddNote,
    EscapePressed,
    BackgroundPressed,
    ClickCatcherPressed,
    RefreshCatchers,
    OpenRescueBoard(SurfaceId),
    CloseRescueBoard,
    MoveNoteHere(Uuid),
    MoveAllHere(String),
    NewNote(SurfaceId),
    Select(Uuid),
    ToggleEdit(Uuid),
    TogglePin(Uuid),
    DeleteNote(Uuid),
    DragStart(SurfaceId, Uuid),
    ResizeStart(SurfaceId, Uuid),
    EditorAction(Uuid, text_editor::Action),
    LinkClicked(String),
    CursorMoved(window::Id, Point),
    CursorEntered(window::Id),
    CursorReleased(window::Id),
    CursorLeft(window::Id),
    ConfirmLeave(u64),
    SaveNote(Uuid),
    NoteSaved(Uuid, Result<(), String>),
}

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub palette: Palette,
    pub notes: Vec<Note>,
    pub outputs: Outputs,
    pub active: Option<Uuid>,
    drag: Option<Drag>,
    resize: Option<Resize>,
    cursor: HashMap<window::Id, Point>,
    pointer_over: Option<window::Id>,
    save_handles: HashMap<Uuid, iced::task::Handle>,
    pending_leave: Option<PendingLeave>,
    leave_token: u64,
    last_config_error: Option<String>,
    catchers: Vec<Catcher>,
    output_geometries: HashMap<String, OutputRect>,
    board: Option<BoardSurface>,
    /// Last input region pushed to each output surface, so we only re-apply it
    /// when it actually changes.
    last_regions: HashMap<SurfaceId, Option<Vec<InputRegionRect>>>,
}

impl App {
    pub fn new(config: Config, config_path: PathBuf) -> (Self, Task<Message>) {
        let notes_path = config.notes_path();
        // Only seed a welcome note the first time, when the notes directory
        // does not exist yet — not every launch that happens to have no notes.
        let first_run = !notes_path.exists();
        if let Err(e) = store::ensure_dir(&notes_path) {
            log::error!("Failed to create notes dir {}: {e}", notes_path.display());
        }

        let mut notes = store::load_notes(&notes_path, &config);
        if first_run && notes.is_empty() {
            let welcome = Note::create(&notes_path, &config, 80.0, 80.0, None);
            if let Err(e) = std::fs::write(&welcome.path, welcome.to_file_string()) {
                log::warn!("Failed to write welcome note: {e}");
            }
            notes.push(welcome);
        }

        let palette = Palette::from_theme(config.theme);
        let mut outputs = Outputs::new(config.layer);
        let top_surfaces = outputs.ensure_top_surfaces::<Message>();
        log::info!("Using theme {:?}", config.theme);

        (
            Self {
                config,
                config_path,
                palette,
                notes,
                outputs,
                active: None,
                drag: None,
                resize: None,
                cursor: HashMap::new(),
                pointer_over: None,
                save_handles: HashMap::new(),
                pending_leave: None,
                leave_token: 0,
                last_config_error: None,
                catchers: Vec::new(),
                output_geometries: geometry::enumerate(),
                board: None,
                last_regions: HashMap::new(),
            },
            top_surfaces,
        )
    }

    pub fn theme(&self) -> Theme {
        self.config.theme.iced()
    }

    pub fn scale_factor(&self) -> f64 {
        self.config.scale_factor
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        Task::batch(vec![
            task,
            self.prune_catchers(),
            self.prune_board(),
            self.sync_input_regions(),
        ])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OutputEvent(event) => self.handle_output_event(event),
            Message::ConfigChanged => self.reload_config(),
            Message::NotesChanged => {
                self.reload_notes();
                Task::none()
            }
            Message::ToggleLayer => self.toggle_layer(),
            Message::AddNote => self.add_note(),
            Message::EscapePressed => {
                if self.board.is_some() {
                    self.close_rescue_board()
                } else {
                    self.exit_editing()
                }
            }
            Message::BackgroundPressed | Message::ClickCatcherPressed => self.exit_editing(),
            Message::RefreshCatchers => self.refresh_catchers(),
            Message::OpenRescueBoard(surface) => self.open_rescue_board(surface),
            Message::CloseRescueBoard => self.close_rescue_board(),
            Message::MoveNoteHere(id) => self.move_note_here(id),
            Message::MoveAllHere(group) => self.move_all_here(group),
            Message::NewNote(surface) => self.new_note(surface),
            Message::Select(id) => self.select(id),
            Message::ToggleEdit(id) => self.toggle_edit(id),
            Message::TogglePin(id) => self.toggle_pin(id),
            Message::DeleteNote(id) => self.delete_note(id),
            Message::DragStart(surface, id) => {
                self.drag_start(surface, id);
                Task::none()
            }
            Message::ResizeStart(surface, id) => {
                self.resize_start(surface, id);
                Task::none()
            }
            Message::EditorAction(id, action) => self.editor_action(id, action),
            Message::LinkClicked(url) => {
                self.open_link(&url);
                Task::none()
            }
            Message::CursorMoved(window, point) => self.cursor_moved(window, point),
            Message::CursorReleased(window) => self.cursor_released(window),
            Message::CursorEntered(window) => self.cursor_entered(window),
            Message::CursorLeft(window) => self.cursor_left(window),
            Message::ConfirmLeave(token) => self.confirm_leave(token),
            Message::SaveNote(id) => self.save_task(id),
            Message::NoteSaved(id, result) => {
                if let Err(e) = result {
                    log::error!("Failed to save note {id}: {e}");
                }
                if let Some(note) = self.find_note_mut(id) {
                    note.dirty = false;
                }
                Task::none()
            }
        }
    }

    pub fn view(&self, id: SurfaceId) -> Element<'_, Message> {
        if let Some(board) = &self.board
            && board.surface_id == id
        {
            let groups: Vec<(String, Vec<&Note>)> = self
                .orphan_groups()
                .into_iter()
                .map(|(name, ids)| {
                    let notes = ids.iter().filter_map(|id| self.find_note(*id)).collect();
                    (name, notes)
                })
                .collect();
            let surface_size = board
                .output_name
                .as_deref()
                .and_then(|name| self.outputs.entries.iter().find(|entry| entry.name == name))
                .and_then(|entry| entry.logical_size)
                .map(|(w, h)| (w as f32, h as f32));
            return widgets::rescue_board(&groups, &self.palette, &self.config, surface_size);
        }

        if self.catchers.iter().any(|catcher| catcher.surface_id == id) {
            return mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::ClickCatcherPressed)
                .on_right_press(Message::ClickCatcherPressed)
                .into();
        }

        let Some((entry, kind)) = self.outputs.entry_for_surface(id) else {
            return Space::new().width(Length::Fill).height(Length::Fill).into();
        };
        let surface_name = entry.name.clone();
        let surface_size = entry.logical_size;
        let raised_surface = kind == SurfaceKind::Top;

        // The bottom surface fills the desktop so a double-click can create a
        // note. The raised surface only hosts its notes, so it uses a plain
        // fullscreen spacer to give the stack the output's size for `pin`.
        let mut stack = if raised_surface {
            Stack::new().push(Space::new().width(Length::Fill).height(Length::Fill))
        } else {
            Stack::new().push(
                mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::BackgroundPressed)
                    .on_right_press(Message::BackgroundPressed)
                    .on_double_click(Message::NewNote(id)),
            )
        };

        for note in &self.notes {
            if self.target_output(note).as_deref() != Some(surface_name.as_str()) {
                continue;
            }
            if note_is_raised(note) != raised_surface {
                continue;
            }
            // While a note is dragged off the edge (cross-monitor drag) it can be
            // fully outside the surface; skip it so we don't lay out a widget at
            // an out-of-range position.
            if let Some((width, height)) = surface_size {
                let (width, height) = (width as f32, height as f32);
                if note.meta.x >= width
                    || note.meta.y >= height
                    || note.meta.x + note.meta.width <= 0.0
                    || note.meta.y + note.meta.height <= 0.0
                {
                    continue;
                }
            }
            let selected = self.active == Some(note.meta.id);
            let card = widgets::note_card(note, id, selected, &self.config, &self.palette);
            stack = stack.push(pin(card).x(note.meta.x).y(note.meta.y));
        }

        // Bottom-right button opening the rescue board, only when there are
        // notes stranded on disconnected monitors. Placed by a fullscreen
        // container aligned to the corner so it never depends on button size.
        let orphan_count: usize = self.orphan_groups().iter().map(|(_, ids)| ids.len()).sum();
        if orphan_count > 0 && !raised_surface {
            let button = widgets::rescue_button(id, orphan_count, &self.palette);
            stack = stack.push(
                container(button)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(Padding {
                        top: 0.0,
                        right: 16.0,
                        bottom: 8.0,
                        left: 0.0,
                    })
                    .align_x(alignment::Horizontal::Right)
                    .align_y(alignment::Vertical::Bottom),
            );
        }

        stack.into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            iced::output_events().map(Message::OutputEvent),
            store::subscription(self.config.notes_path()).map(|()| Message::NotesChanged),
            config::subscription(&self.config_path).map(|()| Message::ConfigChanged),
            crate::ipc::subscription().map(|command| match command {
                crate::ipc::IpcCommand::ToggleLayer => Message::ToggleLayer,
                crate::ipc::IpcCommand::AddNote => Message::AddNote,
            }),
            listen_with(|event, _status, window| match event {
                iced::event::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::CursorMoved(window, position))
                }
                iced::event::Event::Mouse(mouse::Event::CursorEntered) => {
                    Some(Message::CursorEntered(window))
                }
                iced::event::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::CursorReleased(window))
                }
                iced::event::Event::Mouse(mouse::Event::CursorLeft) => {
                    Some(Message::CursorLeft(window))
                }
                iced::event::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::EscapePressed),
                _ => None,
            }),
        ])
    }

    // -- notes -------------------------------------------------------------

    fn find_note(&self, id: Uuid) -> Option<&Note> {
        self.notes.iter().find(|note| note.meta.id == id)
    }

    fn find_note_mut(&mut self, id: Uuid) -> Option<&mut Note> {
        self.notes.iter_mut().find(|note| note.meta.id == id)
    }

    /// The output a note should render on. Notes assigned to a disconnected
    /// monitor are *not* rendered normally (they would land at meaningless
    /// coordinates); they are surfaced through the rescue board instead.
    fn target_output(&self, note: &Note) -> Option<String> {
        match note.meta.output.as_deref() {
            Some(name) if self.outputs.has_name(name) => Some(name.to_owned()),
            Some(_) => None,
            None => self.outputs.primary_name().map(str::to_owned),
        }
    }

    /// Notes grouped by disconnected monitor, in monitor-name order.
    fn orphan_groups(&self) -> Vec<(String, Vec<Uuid>)> {
        let mut groups: std::collections::BTreeMap<String, Vec<Uuid>> =
            std::collections::BTreeMap::new();
        for note in &self.notes {
            if let Some(name) = note.meta.output.as_deref()
                && !self.outputs.has_name(name)
            {
                groups
                    .entry(name.to_owned())
                    .or_default()
                    .push(note.meta.id);
            }
        }
        groups.into_iter().collect()
    }

    /// The output name of the surface backing the given window id, if it is one
    /// of ours.
    fn output_name_for_window(&self, window: window::Id) -> Option<String> {
        self.outputs
            .entry_for_window(window)
            .map(|entry| entry.name.clone())
    }

    /// The logical size of the output backing the given window id, if known.
    fn output_size_for_window(&self, window: window::Id) -> Option<(f32, f32)> {
        self.outputs
            .entry_for_window(window)
            .and_then(|entry| entry.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
    }

    /// Whether the last known cursor position on this surface is outside it,
    /// which (during an implicit grab) means the pointer is on another monitor.
    fn pointer_outside_window(&self, window: window::Id) -> bool {
        let Some(point) = self.cursor.get(&window).copied() else {
            return false;
        };
        let Some((width, height)) = self.output_size_for_window(window) else {
            return false;
        };
        point.x < 0.0 || point.y < 0.0 || point.x > width || point.y > height
    }

    /// Start (or restart) the grace period after which a drag that left a
    /// surface is considered dropped.
    fn start_leave_grace(&mut self, window: window::Id) -> Task<Message> {
        self.leave_token += 1;
        let token = self.leave_token;
        self.pending_leave = Some(PendingLeave { window, token });
        Task::perform(
            async move {
                tokio::time::sleep(LEAVE_GRACE).await;
            },
            move |()| Message::ConfirmLeave(token),
        )
    }

    fn output_bounds_for_note(&self, id: Uuid) -> Option<(f32, f32)> {
        let note = self.find_note(id)?;
        let target = self.target_output(note)?;
        let entry = self.outputs.entries.iter().find(|e| e.name == target)?;
        entry.logical_size.map(|(w, h)| (w as f32, h as f32))
    }

    /// Pull a note fully back inside its output, e.g. after a drag that was
    /// released off-surface without transferring to another monitor.
    fn clamp_note_into_bounds(&mut self, id: Uuid) {
        let Some((ow, oh)) = self.output_bounds_for_note(id) else {
            return;
        };
        if let Some(note) = self.find_note_mut(id) {
            note.meta.x = note.meta.x.clamp(0.0, (ow - note.meta.width).max(0.0));
            note.meta.y = note.meta.y.clamp(0.0, (oh - note.meta.height).max(0.0));
        }
    }

    // -- click catchers ----------------------------------------------------

    fn note_output_id(&self, id: Uuid) -> Option<OutputId> {
        let note = self.find_note(id)?;
        let target = self.target_output(note)?;
        self.outputs
            .entries
            .iter()
            .find(|entry| entry.name == target)
            .and_then(|entry| entry.output_id)
    }

    fn editing_note(&self) -> Option<Uuid> {
        self.notes
            .iter()
            .find(|note| note.editing)
            .map(|note| note.meta.id)
    }

    fn output_logical_size(&self, output: Option<OutputId>) -> Option<(f32, f32)> {
        self.outputs
            .entries
            .iter()
            .find(|entry| entry.output_id == output)
            .and_then(|entry| entry.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
    }

    /// The input region for a catcher on `output`: the whole output, except the
    /// edited note's rectangle when the note lives on this output.
    fn catcher_region(&self, output: Option<OutputId>) -> Vec<InputRegionRect> {
        let Some((width, height)) = self.output_logical_size(output) else {
            return Vec::new();
        };
        let Some(note_id) = self.editing_note() else {
            return Vec::new();
        };
        let Some(note) = self.find_note(note_id) else {
            return Vec::new();
        };

        if self.note_output_id(note_id) == output {
            complement_region(
                width,
                height,
                note.meta.x,
                note.meta.y,
                note.meta.width,
                note.meta.height,
            )
        } else {
            vec![InputRegionRect {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
            }]
        }
    }

    fn catcher_settings(output: Option<OutputId>) -> LayerShellSettings {
        LayerShellSettings {
            anchor: Anchor::all(),
            layer: Layer::Overlay,
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
            size: Some((0, 0)),
            margin: (0, 0, 0, 0),
            namespace: CATCHER_NAMESPACE.to_owned(),
            output,
        }
    }

    /// Create a click catcher on every rendered monitor while a note is edited.
    fn rebuild_catchers(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for catcher in self.catchers.drain(..) {
            tasks.push(destroy_layer_surface(catcher.surface_id));
        }

        if self.editing_note().is_none() {
            return Task::batch(tasks);
        }

        for entry in &self.outputs.entries {
            let (surface_id, create) = new_layer_surface(Self::catcher_settings(entry.output_id));
            self.catchers.push(Catcher {
                surface_id,
                output: entry.output_id,
            });
            tasks.push(create);
        }

        // The surfaces do not exist yet in this batch, so set their input
        // regions on the next update (and again once the compositor maps them).
        tasks.push(Task::done(Message::RefreshCatchers));
        Task::batch(tasks)
    }

    fn refresh_catchers(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for catcher in &self.catchers {
            let region = self.catcher_region(catcher.output);
            tasks.push(set_input_region(catcher.surface_id, Some(region)));
        }
        Task::batch(tasks)
    }

    /// Drop the catchers when no note is being edited.
    fn prune_catchers(&mut self) -> Task<Message> {
        if self.editing_note().is_some() || self.catchers.is_empty() {
            return Task::none();
        }
        let mut tasks = Vec::new();
        for catcher in self.catchers.drain(..) {
            tasks.push(destroy_layer_surface(catcher.surface_id));
        }
        Task::batch(tasks)
    }

    /// Keep the catchers in sync after the edited note moved or resized.
    fn sync_catchers(&mut self) -> Task<Message> {
        if self.editing_note().is_none() {
            return self.prune_catchers();
        }
        if self.catchers.len() != self.outputs.entries.len() {
            return self.rebuild_catchers();
        }
        self.refresh_catchers()
    }

    // -- rescue board ------------------------------------------------------

    fn board_settings(output: Option<OutputId>) -> LayerShellSettings {
        LayerShellSettings {
            anchor: Anchor::all(),
            layer: Layer::Overlay,
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            size: Some((0, 0)),
            margin: (0, 0, 0, 0),
            namespace: BOARD_NAMESPACE.to_owned(),
            output,
        }
    }

    fn open_rescue_board(&mut self, surface: SurfaceId) -> Task<Message> {
        if self.board.is_some() {
            return Task::none();
        }

        let output_name = self
            .output_name_for_window(window::Id::from(surface))
            .or_else(|| self.outputs.primary_name().map(str::to_owned));
        let output_id = output_name.as_deref().and_then(|name| {
            self.outputs
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .and_then(|entry| entry.output_id)
        });

        let (surface_id, create) = new_layer_surface(Self::board_settings(output_id));
        self.board = Some(BoardSurface {
            surface_id,
            output_name,
        });

        // Leave edit mode so the board isn't fighting the click catcher.
        let exit = self.exit_editing();
        Task::batch(vec![create, exit])
    }

    fn close_rescue_board(&mut self) -> Task<Message> {
        match self.board.take() {
            Some(board) => destroy_layer_surface(board.surface_id),
            None => Task::none(),
        }
    }

    fn prune_board(&mut self) -> Task<Message> {
        let Some(board) = self.board.as_ref() else {
            return Task::none();
        };
        let output_gone = board
            .output_name
            .as_deref()
            .is_some_and(|name| !self.outputs.has_name(name));
        if output_gone || self.orphan_groups().is_empty() {
            self.close_rescue_board()
        } else {
            Task::none()
        }
    }

    // -- layer -------------------------------------------------------------

    /// Toggle the note surfaces between `Bottom` (behind windows) and `Top`
    /// (above windows). The change is transient: it is not written back to the
    /// config file, so a config reload or restart restores the configured layer.
    fn toggle_layer(&mut self) -> Task<Message> {
        let layer = match self.config.layer {
            config::Layer::Top => config::Layer::Bottom,
            _ => config::Layer::Top,
        };
        log::info!("Toggling layer to {layer:?}");
        self.config.layer = layer;
        // `set_layer` resets each surface's input region to the full surface.
        // Forget the cached regions so `sync_input_regions` pushes them again,
        // otherwise a surface whose desired region is unchanged keeps the full
        // region and swallows every click on the raised layer.
        self.last_regions.clear();
        self.outputs.apply_layer(layer)
    }

    /// The input region each output surface should have for the current layer.
    ///
    /// On `Bottom` the fullscreen surface keeps its whole input region (so
    /// double-clicking empty desktop can create a note). On `Top`/`Overlay` it
    /// would sit above normal windows and swallow every click, so the region is
    /// limited to the notes (and the rescue button) and clicks elsewhere fall
    /// through to whatever is below.
    fn desired_input_regions(&self) -> HashMap<SurfaceId, Option<Vec<InputRegionRect>>> {
        let orphan_count: usize = self.orphan_groups().iter().map(|(_, ids)| ids.len()).sum();

        let mut regions = HashMap::new();
        for entry in &self.outputs.entries {
            let bottom = match self.config.layer {
                config::Layer::Bottom => None,
                config::Layer::Background => Some(Vec::new()),
                config::Layer::Top | config::Layer::Overlay => {
                    Some(self.surface_input_region(entry, false, orphan_count))
                }
            };
            regions.insert(entry.surface_id, bottom);

            // The raised surface never covers the desktop, so its region is
            // always limited to the notes it renders.
            if let Some(top) = entry.top_surface {
                regions.insert(top, Some(self.surface_input_region(entry, true, 0)));
            }
        }
        regions
    }

    /// Rectangles of everything clickable on `entry` while notes are raised.
    fn surface_input_region(
        &self,
        entry: &crate::outputs::OutputEntry,
        raised: bool,
        orphan_count: usize,
    ) -> Vec<InputRegionRect> {
        let Some((width, height)) = entry.logical_size.map(|(w, h)| (w as f32, h as f32)) else {
            return Vec::new();
        };

        let mut rects = Vec::new();
        for note in &self.notes {
            if self.target_output(note).as_deref() != Some(entry.name.as_str()) {
                continue;
            }
            if note_is_raised(note) != raised {
                continue;
            }
            if let Some(rect) = clip_rect(
                width,
                height,
                note.meta.x,
                note.meta.y,
                note.meta.width,
                note.meta.height,
            ) {
                rects.push(rect);
            }
        }

        // The rescue button pinned to the bottom-right corner of the bottom
        // surface.
        if !raised
            && orphan_count > 0
            && let Some(rect) = clip_rect(width, height, width - 90.0, height - 60.0, 90.0, 60.0)
        {
            rects.push(rect);
        }

        rects
    }

    /// Push each output surface's input region to the compositor, but only when
    /// it changed since the last update.
    fn sync_input_regions(&mut self) -> Task<Message> {
        // Regions only matter between gestures; updating them on every pointer
        // motion would spam the compositor during a drag.
        if self.drag.is_some() || self.resize.is_some() {
            return Task::none();
        }

        let desired = self.desired_input_regions();
        let mut tasks = Vec::new();
        for (surface, region) in &desired {
            if self.last_regions.get(surface) != Some(region) {
                tasks.push(set_input_region(*surface, region.clone()));
            }
        }
        self.last_regions = desired;
        Task::batch(tasks)
    }

    /// Cascade position for a note moved onto `target_name`.
    fn cascade_position(&self, target_name: &str) -> (f32, f32) {
        let count = self
            .notes
            .iter()
            .filter(|note| note.meta.output.as_deref() == Some(target_name))
            .count();
        let (ow, oh) = self
            .outputs
            .entries
            .iter()
            .find(|entry| entry.name == target_name)
            .and_then(|entry| entry.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or((1920.0, 1080.0));

        let offset = (count % 10) as f32 * 28.0;
        let x = (80.0 + offset).min((ow - self.config.default_width).max(0.0));
        let y = (80.0 + offset).min((oh - self.config.default_height).max(0.0));
        (x, y)
    }

    fn move_note_here(&mut self, id: Uuid) -> Task<Message> {
        let Some(target) = self.board.as_ref().and_then(|b| b.output_name.clone()) else {
            return Task::none();
        };
        let (x, y) = self.cascade_position(&target);
        if let Some(note) = self.find_note_mut(id) {
            note.meta.output = Some(target);
            note.meta.x = x;
            note.meta.y = y;
            note.dirty = true;
        }
        self.save_task(id)
    }

    fn move_all_here(&mut self, group: String) -> Task<Message> {
        let ids: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.meta.output.as_deref() == Some(group.as_str()))
            .map(|note| note.meta.id)
            .collect();
        Task::batch(
            ids.into_iter()
                .map(|id| self.move_note_here(id))
                .collect::<Vec<_>>(),
        )
    }

    fn select(&mut self, id: Uuid) -> Task<Message> {
        let mut tasks = Vec::new();
        let others: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.editing && note.meta.id != id)
            .map(|note| note.meta.id)
            .collect();
        for other in others {
            if let Some(note) = self.find_note_mut(other) {
                note.editing = false;
            }
            tasks.push(self.save_task(other));
        }
        self.active = Some(id);
        Task::batch(tasks)
    }

    fn toggle_edit(&mut self, id: Uuid) -> Task<Message> {
        if self.find_note(id).is_some_and(|note| note.editing) {
            return self.exit_editing();
        }

        let mut tasks = Vec::new();
        let others: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.editing)
            .map(|note| note.meta.id)
            .collect();
        for other in others {
            if let Some(note) = self.find_note_mut(other) {
                note.editing = false;
            }
            tasks.push(self.save_task(other));
        }

        if let Some(note) = self.find_note_mut(id) {
            note.editor_mut();
            note.editing = true;
            // Editing always goes to the raised surface, so any deferred move
            // is moot.
            note.raised_override = None;
        }
        self.active = Some(id);
        tasks.push(focus_editor(widgets::editor_id(id)));
        tasks.push(self.rebuild_catchers());
        Task::batch(tasks)
    }

    /// Pin or unpin a note, moving it between the bottom and raised layers.
    /// Editing keeps a note raised regardless of this flag. Pinning is
    /// runtime-only and is never written to the note file.
    ///
    /// If the pointer is hovering the note, the surface move is deferred until
    /// it leaves: changing the surface under a stationary pointer leaves the
    /// compositor's focus on the old surface, so a second click would be lost.
    fn toggle_pin(&mut self, id: Uuid) -> Task<Message> {
        let Some(note) = self.find_note(id) else {
            return Task::none();
        };
        let was_raised = note_is_raised(note);
        let cursor_over = self.cursor_within_note(note);

        let note = self.find_note_mut(id).expect("note just looked up");
        note.pinned = !note.pinned;
        note.raised_override =
            (note_wants_raised(note) != was_raised && cursor_over).then_some(was_raised);
        Task::none()
    }

    /// The surface a note is currently rendered on.
    fn note_surface(&self, note: &Note) -> Option<SurfaceId> {
        let entry = self
            .target_output(note)
            .and_then(|name| self.outputs.entries.iter().find(|entry| entry.name == name))?;
        if note_is_raised(note) {
            entry.top_surface
        } else {
            Some(entry.surface_id)
        }
    }

    /// Whether the last known pointer position is inside the note's rectangle,
    /// on the surface the note is currently rendered on.
    fn cursor_within_note(&self, note: &Note) -> bool {
        let Some(surface) = self.note_surface(note) else {
            return false;
        };
        let Some(point) = self.cursor.get(&window::Id::from(surface)) else {
            return false;
        };
        point.x >= note.meta.x
            && point.x <= note.meta.x + note.meta.width
            && point.y >= note.meta.y
            && point.y <= note.meta.y + note.meta.height
    }

    /// Release deferred layer moves once the pointer is no longer over the
    /// note, allowing it to move to its desired layer.
    fn release_layer_overrides(&mut self, window: window::Id, point: Point) {
        let ids: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.raised_override.is_some())
            .filter(|note| {
                let on_window = self
                    .note_surface(note)
                    .is_some_and(|surface| window::Id::from(surface) == window);
                let inside = point.x >= note.meta.x
                    && point.x <= note.meta.x + note.meta.width
                    && point.y >= note.meta.y
                    && point.y <= note.meta.y + note.meta.height;
                !(on_window && inside)
            })
            .map(|note| note.meta.id)
            .collect();
        for id in ids {
            if let Some(note) = self.find_note_mut(id) {
                note.raised_override = None;
            }
        }
    }

    /// Release every deferred layer move on `window`, e.g. when the pointer
    /// leaves its surface entirely.
    fn release_overrides_on(&mut self, window: window::Id) {
        let ids: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.raised_override.is_some())
            .filter(|note| {
                self.note_surface(note)
                    .is_some_and(|surface| window::Id::from(surface) == window)
            })
            .map(|note| note.meta.id)
            .collect();
        for id in ids {
            if let Some(note) = self.find_note_mut(id) {
                note.raised_override = None;
            }
        }
    }

    fn exit_editing(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for catcher in self.catchers.drain(..) {
            tasks.push(destroy_layer_surface(catcher.surface_id));
        }
        let editing: Vec<Uuid> = self
            .notes
            .iter()
            .filter(|note| note.editing)
            .map(|note| note.meta.id)
            .collect();
        for id in editing {
            if let Some(note) = self.find_note_mut(id) {
                note.editing = false;
            }
            tasks.push(self.save_task(id));
        }
        self.active = None;
        for note in &mut self.notes {
            note.confirm_delete = false;
        }
        Task::batch(tasks)
    }

    fn new_note(&mut self, surface: SurfaceId) -> Task<Message> {
        let output = self
            .outputs
            .get(surface)
            .map(|entry| entry.name.clone())
            .or_else(|| self.outputs.primary_name().map(str::to_owned));

        let (x, y) = self.new_note_position(surface);
        self.create_note(output, x, y)
    }

    /// Create a note from IPC, on the primary output. There is no pointer on a
    /// layer surface to anchor to, so it is cascaded like a fresh note.
    fn add_note(&mut self) -> Task<Message> {
        let Some(entry) = self.outputs.entries.first() else {
            return Task::none();
        };
        let output = Some(entry.name.clone());
        let (x, y) = self.cascade_position(&entry.name);
        self.create_note(output, x, y)
    }

    /// Insert a note, drop straight into editing it and persist it.
    fn create_note(&mut self, output: Option<String>, x: f32, y: f32) -> Task<Message> {
        let note = Note::create(&self.config.notes_path(), &self.config, x, y, output);
        let id = note.meta.id;
        self.notes.push(note);

        if let Some(note) = self.find_note_mut(id) {
            note.editor_mut();
            note.editing = true;
        }
        self.active = Some(id);

        Task::batch(vec![
            self.save_task(id),
            focus_editor(widgets::editor_id(id)),
            self.rebuild_catchers(),
        ])
    }

    fn new_note_position(&self, surface: SurfaceId) -> (f32, f32) {
        let window = window::Id::from(surface);
        let width = self.config.default_width;
        let height = self.config.default_height;

        let base = self.cursor.get(&window).copied().unwrap_or_else(|| {
            let offset = (self.notes.len() as f32 % 8.0) * 28.0;
            Point::new(80.0 + offset, 80.0 + offset)
        });

        let (mut x, mut y) = (base.x.max(0.0), base.y.max(0.0));
        if let Some((ow, oh)) = self
            .outputs
            .get(surface)
            .and_then(|entry| entry.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
        {
            x = x.min((ow - width).max(0.0));
            y = y.min((oh - height).max(0.0));
        }
        (x, y)
    }

    fn delete_note(&mut self, id: Uuid) -> Task<Message> {
        let Some(index) = self.notes.iter().position(|note| note.meta.id == id) else {
            return Task::none();
        };

        if !self.notes[index].confirm_delete {
            for note in &mut self.notes {
                note.confirm_delete = false;
            }
            self.notes[index].confirm_delete = true;
            return Task::none();
        }

        let note = self.notes.remove(index);
        self.save_handles.remove(&id);
        if self.active == Some(id) {
            self.active = None;
        }
        if self.drag.is_some_and(|drag| drag.note == id) {
            self.drag = None;
        }
        if self.resize.is_some_and(|resize| resize.note == id) {
            self.resize = None;
        }

        Task::perform(store::remove_note(note.path), move |result| {
            Message::NoteSaved(id, result)
        })
    }

    fn editor_action(&mut self, id: Uuid, action: text_editor::Action) -> Task<Message> {
        let is_edit = action.is_edit();
        if let Some(note) = self.find_note_mut(id) {
            note.editor_mut().perform(action);
            if is_edit {
                note.dirty = true;
            }
        }
        if is_edit {
            self.schedule_save(id)
        } else {
            Task::none()
        }
    }

    fn drag_start(&mut self, surface: SurfaceId, id: Uuid) {
        let window = window::Id::from(surface);
        let (start_pos, start_cursor) = {
            let Some(note) = self.find_note(id) else {
                return;
            };
            let start_pos = Point::new(note.meta.x, note.meta.y);
            let start_cursor = self.cursor.get(&window).copied().unwrap_or(start_pos);
            (start_pos, start_cursor)
        };
        self.active = Some(id);
        self.drag = Some(Drag {
            window,
            note: id,
            start_cursor,
            start_pos,
            released: false,
        });
    }

    fn resize_start(&mut self, surface: SurfaceId, id: Uuid) {
        let window = window::Id::from(surface);
        let (start_size, start_cursor) = {
            let Some(note) = self.find_note(id) else {
                return;
            };
            let start_size = Size::new(note.meta.width, note.meta.height);
            let start_cursor = self
                .cursor
                .get(&window)
                .copied()
                .unwrap_or(Point::new(note.meta.x, note.meta.y));
            (start_size, start_cursor)
        };
        self.active = Some(id);
        self.resize = Some(Resize {
            window,
            note: id,
            start_cursor,
            start_size,
        });
    }

    fn cursor_moved(&mut self, window: window::Id, point: Point) -> Task<Message> {
        self.cursor.insert(window, point);
        self.release_layer_overrides(window, point);

        // Hand an in-flight drag/resize over to this surface if it started on a
        // different one (the pointer crossed to another monitor).
        if self.output_name_for_window(window).is_some() {
            if self.drag.is_some() || self.resize.is_some() {
                self.pending_leave = None;
            }
            if self.drag.is_some_and(|drag| drag.window != window) {
                self.retarget_drag(window, point);
                // A drag whose button was already released is finished as soon
                // as it lands on the new monitor.
                if let Some(drag) = self.drag.filter(|drag| drag.released) {
                    self.drag = None;
                    self.pending_leave = None;
                    let save = self.save_task(drag.note);
                    let sync = self.sync_catchers();
                    return Task::batch(vec![save, sync]);
                }
            }
            if self.resize.is_some_and(|resize| resize.window != window) {
                self.retarget_resize(window, point);
            }
        }

        // Follow the pointer exactly, even outside the surface, so the note
        // visibly slides off the edge toward the neighbouring monitor.
        if let Some(drag) = self
            .drag
            .filter(|drag| drag.window == window && !drag.released)
            && let Some(note) = self.find_note_mut(drag.note)
        {
            note.meta.x = drag.start_pos.x + (point.x - drag.start_cursor.x);
            note.meta.y = drag.start_pos.y + (point.y - drag.start_cursor.y);
        }

        if let Some(resize) = self.resize.filter(|resize| resize.window == window) {
            let bounds = self.output_bounds_for_note(resize.note);
            if let Some(note) = self.find_note_mut(resize.note) {
                let mut width =
                    (resize.start_size.width + (point.x - resize.start_cursor.x)).max(140.0);
                let mut height =
                    (resize.start_size.height + (point.y - resize.start_cursor.y)).max(100.0);
                if let Some((ow, oh)) = bounds {
                    width = width.min((ow - note.meta.x).max(140.0));
                    height = height.min((oh - note.meta.y).max(100.0));
                }
                note.meta.width = width;
                note.meta.height = height;
            }
        }

        Task::none()
    }

    fn cursor_entered(&mut self, window: window::Id) -> Task<Message> {
        self.pointer_over = Some(window);
        // Forget any stale position for this surface; a fresh motion will set it.
        self.cursor.remove(&window);

        if self.output_name_for_window(window).is_none() {
            return Task::none();
        }

        // A drag/resize that started elsewhere keeps a grace period running, so a
        // fresh motion can re-home it, otherwise it is finalized geometrically.
        if let Some(drag) = self.drag.filter(|drag| drag.window != window) {
            return self.start_leave_grace(drag.window);
        }
        if let Some(resize) = self.resize.filter(|resize| resize.window != window) {
            return self.start_leave_grace(resize.window);
        }
        if self.drag.is_some() || self.resize.is_some() {
            self.pending_leave = None;
        }
        Task::none()
    }

    /// Re-home an in-flight drag onto another of our surfaces, preserving the
    /// grab offset so the note does not jump under the cursor.
    fn retarget_drag(&mut self, window: window::Id, point: Point) {
        let Some(drag) = self.drag else {
            return;
        };
        self.pending_leave = None;

        let grab_x = drag.start_cursor.x - drag.start_pos.x;
        let grab_y = drag.start_cursor.y - drag.start_pos.y;
        let start_pos = Point::new(point.x - grab_x, point.y - grab_y);

        self.place_note(drag.note, window, start_pos);

        self.drag = Some(Drag {
            window,
            note: drag.note,
            start_cursor: point,
            start_pos,
            released: drag.released,
        });
    }

    /// Move a note to `window`'s output at the given logical position, clamped
    /// to that output.
    fn place_note(&mut self, note_id: Uuid, window: window::Id, position: Point) {
        let name = self.output_name_for_window(window);
        let bounds = self.output_size_for_window(window);
        if let Some(note) = self.find_note_mut(note_id) {
            if let Some(name) = name {
                note.meta.output = Some(name);
            }
            let mut x = position.x;
            let mut y = position.y;
            if let Some((ow, oh)) = bounds {
                x = x.clamp(0.0, (ow - note.meta.width).max(0.0));
                y = y.clamp(0.0, (oh - note.meta.height).max(0.0));
            } else {
                x = x.max(0.0);
                y = y.max(0.0);
            }
            note.meta.x = x;
            note.meta.y = y;
        }
    }

    /// Approximate the note's position on `target` from its position on
    /// `source`, mirroring how far it crossed the shared edge. Used when no
    /// fresh pointer position is available on the target monitor.
    fn geometric_position(
        &self,
        note_id: Uuid,
        source: window::Id,
        target: window::Id,
    ) -> Option<Point> {
        let note = self.find_note(note_id)?;
        let src = self.output_size_for_window(source)?;
        let dst = self.output_size_for_window(target)?;
        let (x, y) = map_across_edges(
            src,
            dst,
            (note.meta.x, note.meta.y, note.meta.width, note.meta.height),
        )?;
        Some(Point::new(x, y))
    }

    /// Precise position of a note on `target_window` using the cached output
    /// layout, or `None` if the layout is unavailable.
    fn geometry_position_for(
        &self,
        note_id: Uuid,
        source_window: window::Id,
        target_window: window::Id,
    ) -> Option<Point> {
        let source_name = self.output_name_for_window(source_window)?;
        let target_name = self.output_name_for_window(target_window)?;
        let source_rect = *self.output_geometries.get(&source_name)?;
        let target_rect = *self.output_geometries.get(&target_name)?;
        let note = self.find_note(note_id)?;
        Some(Point::new(
            source_rect.x as f32 + note.meta.x - target_rect.x as f32,
            source_rect.y as f32 + note.meta.y - target_rect.y as f32,
        ))
    }

    /// Finish a released drag on `target`, preferring a fresh pointer position
    /// and falling back to a geometric mapping.
    fn finalize_drag_on(&mut self, target: window::Id, drag: Drag) -> Task<Message> {
        let position = match self.cursor.get(&target).copied() {
            Some(point) => {
                let grab_x = drag.start_cursor.x - drag.start_pos.x;
                let grab_y = drag.start_cursor.y - drag.start_pos.y;
                Some(Point::new(point.x - grab_x, point.y - grab_y))
            }
            None => self
                .geometry_position_for(drag.note, drag.window, target)
                .or_else(|| self.geometric_position(drag.note, drag.window, target)),
        };

        if let Some(position) = position {
            self.place_note(drag.note, target, position);
        } else if let Some(name) = self.output_name_for_window(target) {
            if let Some(note) = self.find_note_mut(drag.note) {
                note.meta.output = Some(name);
            }
            self.clamp_note_into_bounds(drag.note);
        }

        self.drag = None;
        self.pending_leave = None;
        let save = self.save_task(drag.note);
        let sync = self.sync_catchers();
        Task::batch(vec![save, sync])
    }

    /// Transfer a drag released outside its surface to whichever output the
    /// pointer is over, using the cached output layout. This does not depend on
    /// the compositor sending `enter`/motion on the target monitor (which it may
    /// only do once the pointer moves again). Returns `None` if the layout is
    /// unknown or the pointer is not over another rendered output.
    fn try_geometry_transfer(&mut self, window: window::Id, drag: Drag) -> Option<Task<Message>> {
        // Some compositors report every output at the same origin; if so the
        // layout is unusable and we fall back to enter/motion.
        let origins: std::collections::HashSet<(i32, i32)> = self
            .output_geometries
            .values()
            .map(|rect| (rect.x, rect.y))
            .collect();
        if self.output_geometries.len() > 1 && origins.len() < 2 {
            return None;
        }

        let source_name = self.output_name_for_window(window)?;
        let source_rect = *self.output_geometries.get(&source_name)?;
        let cursor = self.cursor.get(&window).copied()?;
        let global_x = source_rect.x as f32 + cursor.x;
        let global_y = source_rect.y as f32 + cursor.y;

        let has_name = |name: &str| self.outputs.has_name(name);
        let (target_name, target_rect) = self
            .output_geometries
            .iter()
            .find(|(name, rect)| {
                name.as_str() != source_name.as_str()
                    && has_name(name)
                    && rect.contains(global_x, global_y)
            })
            .map(|(name, rect)| (name.clone(), *rect))?;

        let target_size = self
            .outputs
            .entries
            .iter()
            .find(|entry| entry.name == target_name)
            .and_then(|entry| entry.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or((target_rect.width as f32, target_rect.height as f32));

        let note = self.find_note(drag.note)?;
        let local_x = source_rect.x as f32 + note.meta.x - target_rect.x as f32;
        let local_y = source_rect.y as f32 + note.meta.y - target_rect.y as f32;

        if let Some(note) = self.find_note_mut(drag.note) {
            note.meta.output = Some(target_name);
            note.meta.x = local_x.clamp(0.0, (target_size.0 - note.meta.width).max(0.0));
            note.meta.y = local_y.clamp(0.0, (target_size.1 - note.meta.height).max(0.0));
        }

        self.drag = None;
        self.pending_leave = None;
        let save = self.save_task(drag.note);
        let sync = self.sync_catchers();
        Some(Task::batch(vec![save, sync]))
    }

    fn refresh_output_geometries(&mut self) {
        self.output_geometries = geometry::enumerate();
    }

    fn retarget_resize(&mut self, window: window::Id, point: Point) {
        let Some(resize) = self.resize else {
            return;
        };
        self.pending_leave = None;

        if let Some(name) = self.output_name_for_window(window)
            && let Some(note) = self.find_note_mut(resize.note)
        {
            note.meta.output = Some(name);
        }

        self.resize = Some(Resize {
            window,
            note: resize.note,
            start_cursor: point,
            start_size: resize.start_size,
        });
    }

    fn cursor_released(&mut self, window: window::Id) -> Task<Message> {
        // Releasing on a different surface transfers the drag/resize there.
        if self.output_name_for_window(window).is_some() {
            if let Some(drag) = self.drag.filter(|drag| drag.window != window) {
                return self.finalize_drag_on(window, drag);
            }
            if let Some(resize) = self.resize.filter(|resize| resize.window != window) {
                if let Some(name) = self.output_name_for_window(window)
                    && let Some(note) = self.find_note_mut(resize.note)
                {
                    note.meta.output = Some(name);
                }
                self.clamp_note_into_bounds(resize.note);
                self.resize = None;
                self.pending_leave = None;
                let save = self.save_task(resize.note);
                let sync = self.sync_catchers();
                return Task::batch(vec![save, sync]);
            }
        }

        if let Some(drag) = self.drag.filter(|drag| drag.window == window) {
            // Wayland keeps an implicit pointer grab while a button is held, so
            // the pointer can be on another monitor while we only see
            // out-of-bounds coordinates. Resolve the target from the output
            // layout if we can, so a release without further motion still moves
            // the note; otherwise wait for the target's enter/motion.
            if self.pointer_outside_window(window) {
                if let Some(task) = self.try_geometry_transfer(window, drag) {
                    return task;
                }
                self.drag = Some(Drag {
                    released: true,
                    ..drag
                });
                return self.start_leave_grace(window);
            }
            self.drag = None;
            self.pending_leave = None;
            self.clamp_note_into_bounds(drag.note);
            let save = self.save_task(drag.note);
            let sync = self.sync_catchers();
            return Task::batch(vec![save, sync]);
        }
        if let Some(resize) = self.resize.filter(|resize| resize.window == window) {
            self.resize = None;
            self.pending_leave = None;
            let save = self.save_task(resize.note);
            let sync = self.sync_catchers();
            return Task::batch(vec![save, sync]);
        }
        Task::none()
    }

    /// The pointer left this surface (another monitor, a window, ...). Give an
    /// in-flight drag a grace period to be picked up by another monitor before
    /// treating it as dropped. Unfocusing is click-driven now (click catcher).
    fn cursor_left(&mut self, window: window::Id) -> Task<Message> {
        if self.pointer_over == Some(window) {
            self.pointer_over = None;
        }

        // The pointer is no longer over anything on this surface, so any
        // deferred layer move here can be released.
        self.release_overrides_on(window);

        let dragging = self.drag.is_some_and(|drag| drag.window == window)
            || self.resize.is_some_and(|resize| resize.window == window);

        if dragging {
            self.start_leave_grace(window)
        } else {
            Task::none()
        }
    }

    fn confirm_leave(&mut self, token: u64) -> Task<Message> {
        let Some(pending) = self.pending_leave.filter(|pending| pending.token == token) else {
            return Task::none();
        };
        self.pending_leave = None;

        if let Some(drag) = self.drag.filter(|drag| drag.window == pending.window) {
            // A released drag that landed on another of our surfaces finishes
            // there (with a fresh pointer position if we got one, otherwise a
            // geometric mapping).
            if drag.released
                && let Some(over) = self.pointer_over.filter(|over| *over != pending.window)
                && self.output_name_for_window(over).is_some()
            {
                return self.finalize_drag_on(over, drag);
            }

            // Otherwise, while the pointer is still on one of our surfaces, let
            // the drag continue until it moves or is released there.
            if !drag.released
                && let Some(over) = self.pointer_over
                && over != pending.window
                && self.output_name_for_window(over).is_some()
            {
                return Task::none();
            }

            self.drag = None;
            self.clamp_note_into_bounds(drag.note);
            let save = self.save_task(drag.note);
            let sync = self.sync_catchers();
            return Task::batch(vec![save, sync]);
        }

        if let Some(resize) = self.resize.filter(|resize| resize.window == pending.window) {
            if let Some(over) = self.pointer_over
                && over != pending.window
                && self.output_name_for_window(over).is_some()
            {
                return Task::none();
            }
            self.resize = None;
            let save = self.save_task(resize.note);
            let sync = self.sync_catchers();
            return Task::batch(vec![save, sync]);
        }

        Task::none()
    }

    // -- persistence -------------------------------------------------------

    fn save_task(&mut self, id: Uuid) -> Task<Message> {
        let Some(note) = self.find_note_mut(id) else {
            return Task::none();
        };
        sync_body_from_editor(note);
        note.meta.updated = Local::now();
        note.dirty = false;
        let path = note.path.clone();
        let content = note.to_file_string();
        Task::perform(store::write_note(path, content), move |result| {
            Message::NoteSaved(id, result)
        })
    }

    fn schedule_save(&mut self, id: Uuid) -> Task<Message> {
        if let Some(handle) = self.save_handles.remove(&id) {
            handle.abort();
        }
        let delay = self.config.autosave_debounce();
        let (task, handle) = Task::perform(
            async move {
                tokio::time::sleep(delay).await;
            },
            move |()| Message::SaveNote(id),
        )
        .abortable();
        self.save_handles.insert(id, handle);
        task
    }

    fn reload_notes(&mut self) {
        let loaded = store::load_notes(&self.config.notes_path(), &self.config);
        let mut by_id: HashMap<Uuid, Note> = loaded
            .into_iter()
            .map(|note| (note.meta.id, note))
            .collect();
        let mut merged = Vec::new();

        for note in self.notes.drain(..) {
            if note.editing || note.dirty {
                by_id.remove(&note.meta.id);
                merged.push(note);
            } else if let Some(mut fresh) = by_id.remove(&note.meta.id) {
                fresh.confirm_delete = note.confirm_delete;
                // Pinning is runtime-only, so keep it across a reload.
                fresh.pinned = note.pinned;
                fresh.raised_override = note.raised_override;
                merged.push(fresh);
            }
        }
        merged.extend(by_id.into_values());
        merged.sort_by(|a, b| {
            a.meta
                .created
                .cmp(&b.meta.created)
                .then_with(|| a.meta.id.cmp(&b.meta.id))
        });
        self.notes = merged;

        if let Some(active) = self.active
            && self.find_note(active).is_none()
        {
            self.active = None;
        }
    }

    // -- outputs / config --------------------------------------------------

    fn handle_output_event(&mut self, event: OutputEvent) -> Task<Message> {
        match event {
            OutputEvent::Added(info) => {
                if !config::output_matches(&self.config.outputs, &info.name) {
                    return Task::none();
                }
                let name = info.name.clone();
                let task = self.outputs.add(name.clone(), info.id, info.logical_size);
                for note in &mut self.notes {
                    if note.meta.output.is_none() {
                        note.meta.output = Some(name.clone());
                    }
                }
                self.refresh_output_geometries();
                let rebuild = self.rebuild_catchers();
                Task::batch(vec![task, rebuild])
            }
            OutputEvent::InfoChanged(info) => {
                self.outputs.set_logical_size(info.id, info.logical_size);
                self.refresh_output_geometries();
                self.refresh_catchers()
            }
            OutputEvent::Removed(id) => {
                let task = self.outputs.remove(id);
                self.refresh_output_geometries();
                let rebuild = self.rebuild_catchers();
                Task::batch(vec![task, rebuild])
            }
            OutputEvent::SurfaceEnteredOutput { surface, .. } => {
                // A surface may have been created after we pushed its input
                // region, in which case the command was dropped; forget it so
                // the region is re-applied on this update.
                self.last_regions.remove(&surface);

                // Once a catcher is mapped, (re)apply the input regions.
                if self
                    .catchers
                    .iter()
                    .any(|catcher| catcher.surface_id == surface)
                {
                    self.refresh_catchers()
                } else {
                    Task::none()
                }
            }
            OutputEvent::SurfaceLeftOutput { .. } => Task::none(),
        }
    }

    fn reload_config(&mut self) -> Task<Message> {
        match config::read_config(&self.config_path) {
            Ok(new_config) => {
                self.last_config_error = None;
                let notes_dir_changed = new_config.notes_path() != self.config.notes_path();
                let layer_changed = new_config.layer != self.config.layer;
                self.config = new_config;
                self.palette = Palette::from_theme(self.config.theme);

                let mut tasks = Vec::new();
                if layer_changed {
                    self.last_regions.clear();
                    tasks.push(self.outputs.recreate_all(self.config.layer));
                }
                if notes_dir_changed {
                    let _ = store::ensure_dir(&self.config.notes_path());
                    self.notes = store::load_notes(&self.config.notes_path(), &self.config);
                    self.active = None;
                }
                Task::batch(tasks)
            }
            Err(e) => {
                // Report the full error chain, but only once per distinct
                // failure so a missing/locked file doesn't spam the log.
                let message = format!("{e:#}");
                if self.last_config_error.as_deref() != Some(message.as_str()) {
                    log::warn!("Failed to reload config: {message}");
                    self.last_config_error = Some(message);
                }
                Task::none()
            }
        }
    }

    fn open_link(&self, url: &str) {
        if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
            log::warn!("Failed to open link {url}: {e}");
        }
    }
}

/// Whether a note renders on the raised surface: either explicitly pinned, or
/// temporarily while it is being edited.
fn note_is_raised(note: &Note) -> bool {
    note.editing || note.raised_override.unwrap_or(note.pinned)
}

/// The raised state a note *wants*, ignoring any deferral in progress.
fn note_wants_raised(note: &Note) -> bool {
    note.pinned || note.editing
}

fn sync_body_from_editor(note: &mut Note) {
    if let Some(editor) = &note.editor {
        let text = editor.text();
        if text != note.body {
            note.body = text;
            note.refresh_items();
        }
    }
}

fn focus_editor<M: Send + 'static>(id: iced::widget::Id) -> Task<M> {
    iced_runtime::task::widget(iced::core::widget::operation::focusable::focus::<M>(id)).into()
}

/// Clip a rectangle to a `width`x`height` surface, returning `None` when it
/// lies fully outside it.
fn clip_rect(width: f32, height: f32, x: f32, y: f32, w: f32, h: f32) -> Option<InputRegionRect> {
    let nx = x.max(0.0).min(width);
    let ny = y.max(0.0).min(height);
    let nx2 = (x + w).clamp(nx, width);
    let ny2 = (y + h).clamp(ny, height);
    (nx2 > nx && ny2 > ny).then_some(InputRegionRect {
        x: nx as i32,
        y: ny as i32,
        width: (nx2 - nx) as i32,
        height: (ny2 - ny) as i32,
    })
}

/// Rectangles covering an `width`x`height` area minus the `(x, y, w, h)` hole.
fn complement_region(
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Vec<InputRegionRect> {
    let nx = x.max(0.0).min(width);
    let ny = y.max(0.0).min(height);
    let nx2 = (x + w).clamp(nx, width);
    let ny2 = (y + h).clamp(ny, height);

    fn push(rects: &mut Vec<InputRegionRect>, x: f32, y: f32, w: f32, h: f32) {
        if w > 0.0 && h > 0.0 {
            rects.push(InputRegionRect {
                x: x as i32,
                y: y as i32,
                width: w as i32,
                height: h as i32,
            });
        }
    }

    let mut rects = Vec::new();
    push(&mut rects, 0.0, 0.0, width, ny); // above the note
    push(&mut rects, 0.0, ny2, width, height - ny2); // below the note
    push(&mut rects, 0.0, ny, nx, ny2 - ny); // left of the note
    push(&mut rects, nx2, ny, width - nx2, ny2 - ny); // right of the note
    rects
}

/// Map a `(x, y, w, h)` rectangle from a `src`-sized output onto a `dst`-sized
/// neighbour by mirroring how far it crossed the shared edge (the axis with the
/// largest overflow). Returns `None` if the rectangle is fully inside `src`.
fn map_across_edges(
    src: (f32, f32),
    dst: (f32, f32),
    rect: (f32, f32, f32, f32),
) -> Option<(f32, f32)> {
    let (sw, sh) = src;
    let (tw, th) = dst;
    let (x, y, w, h) = rect;

    let right = x + w - sw;
    let left = -x;
    let bottom = y + h - sh;
    let top = -y;
    let max = right.max(left).max(bottom).max(top);
    if max <= 0.0 {
        return None;
    }

    let (mut nx, mut ny) = (x, y);
    if right >= max {
        nx = x - sw;
    } else if left >= max {
        nx = x + tw;
    } else if bottom >= max {
        ny = y - sh;
    } else {
        ny = y + th;
    }

    Some((
        nx.clamp(0.0, (tw - w).max(0.0)),
        ny.clamp(0.0, (th - h).max(0.0)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_rect_inside_is_unchanged() {
        assert_eq!(
            clip_rect(100.0, 100.0, 10.0, 20.0, 30.0, 40.0),
            Some(InputRegionRect {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            })
        );
    }

    #[test]
    fn clip_rect_partial_is_clamped() {
        assert_eq!(
            clip_rect(100.0, 100.0, 80.0, 90.0, 50.0, 50.0),
            Some(InputRegionRect {
                x: 80,
                y: 90,
                width: 20,
                height: 10,
            })
        );
    }

    #[test]
    fn clip_rect_outside_is_none() {
        assert_eq!(clip_rect(100.0, 100.0, 120.0, 10.0, 30.0, 30.0), None);
        assert_eq!(clip_rect(100.0, 100.0, -40.0, 10.0, 30.0, 30.0), None);
    }

    #[test]
    fn complement_region_splits_around_hole() {
        let rects = complement_region(100.0, 100.0, 20.0, 30.0, 40.0, 50.0);
        assert_eq!(rects.len(), 4);
        // Above and below span the full width.
        assert_eq!(
            rects[0],
            InputRegionRect {
                x: 0,
                y: 0,
                width: 100,
                height: 30
            }
        );
        assert_eq!(
            rects[1],
            InputRegionRect {
                x: 0,
                y: 80,
                width: 100,
                height: 20
            }
        );
        // Left and right flank the hole.
        assert_eq!(
            rects[2],
            InputRegionRect {
                x: 0,
                y: 30,
                width: 20,
                height: 50
            }
        );
        assert_eq!(
            rects[3],
            InputRegionRect {
                x: 60,
                y: 30,
                width: 40,
                height: 50
            }
        );
    }

    #[test]
    fn complement_region_full_hole_is_empty() {
        let rects = complement_region(100.0, 100.0, 0.0, 0.0, 100.0, 100.0);
        assert!(rects.is_empty());
    }

    #[test]
    fn complement_region_clamps_out_of_bounds_hole() {
        let rects = complement_region(100.0, 100.0, 80.0, 90.0, 500.0, 500.0);
        // Only the area above and to the left of the (clamped) hole remains.
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].height, 90); // above
        assert_eq!(rects[1].width, 80); // left
    }

    #[test]
    fn map_across_edges_inside_is_none() {
        assert_eq!(
            map_across_edges(
                (1920.0, 1080.0),
                (2560.0, 1440.0),
                (100.0, 100.0, 260.0, 220.0)
            ),
            None
        );
    }

    #[test]
    fn map_across_edges_mirrors_right_crossing() {
        // Note's right edge is 140px past the source's right edge.
        let mapped = map_across_edges(
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (1800.0, 100.0, 260.0, 220.0),
        );
        // x is clamped to the destination's left edge, y is preserved.
        assert_eq!(mapped, Some((0.0, 100.0)));
    }

    #[test]
    fn map_across_edges_mirrors_left_crossing() {
        // Note crossed the source's left edge by 120px; destination is to the
        // left, so it lands at the destination's right edge.
        let mapped = map_across_edges(
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (-120.0, 100.0, 260.0, 220.0),
        );
        assert_eq!(mapped, Some((2300.0, 100.0)));
    }

    #[test]
    fn map_across_edges_mirrors_bottom_crossing() {
        let mapped = map_across_edges(
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (100.0, 1000.0, 260.0, 220.0),
        );
        assert_eq!(mapped, Some((100.0, 0.0)));
    }
}
