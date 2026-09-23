use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use uuid::Uuid;

use iced::event::listen_with;
use iced::widget::{Space, Stack, mouse_area, pin, text_editor};
use iced::{
    Element, Length, OutputEvent, Point, Size, Subscription, SurfaceId, Task, Theme, keyboard,
    mouse, window,
};

use crate::config::{self, Config};
use crate::note::Note;
use crate::outputs::Outputs;
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

#[derive(Debug, Clone)]
pub enum Message {
    OutputEvent(OutputEvent),
    ConfigChanged,
    NotesChanged,
    EscapePressed,
    BackgroundPressed,
    NewNote(SurfaceId),
    Select(Uuid),
    ToggleEdit(Uuid),
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
}

impl App {
    pub fn new(config: Config, config_path: PathBuf) -> (Self, Task<Message>) {
        let notes_path = config.notes_path();
        if let Err(e) = store::ensure_dir(&notes_path) {
            log::error!("Failed to create notes dir {}: {e}", notes_path.display());
        }

        let mut notes = store::load_notes(&notes_path, &config);
        if notes.is_empty() {
            let welcome = Note::create(&notes_path, &config, 80.0, 80.0, None);
            if let Err(e) = std::fs::write(&welcome.path, welcome.to_file_string()) {
                log::warn!("Failed to write welcome note: {e}");
            }
            notes.push(welcome);
        }

        let palette = Palette::from_theme(config.theme);
        let outputs = Outputs::new(config.layer);
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
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        self.config.theme.iced()
    }

    pub fn scale_factor(&self) -> f64 {
        self.config.scale_factor
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OutputEvent(event) => self.handle_output_event(event),
            Message::ConfigChanged => self.reload_config(),
            Message::NotesChanged => {
                self.reload_notes();
                Task::none()
            }
            Message::EscapePressed | Message::BackgroundPressed => self.exit_editing(),
            Message::NewNote(surface) => self.new_note(surface),
            Message::Select(id) => self.select(id),
            Message::ToggleEdit(id) => self.toggle_edit(id),
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
            Message::CursorEntered(window) => {
                self.cursor_entered(window);
                Task::none()
            }
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
        let Some(entry) = self.outputs.get(id) else {
            return Space::new().width(Length::Fill).height(Length::Fill).into();
        };
        let surface_name = entry.name.clone();

        let mut stack = Stack::new().push(
            mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::BackgroundPressed)
                .on_right_press(Message::BackgroundPressed)
                .on_double_click(Message::NewNote(id)),
        );

        for note in &self.notes {
            if self.target_output(note).as_deref() != Some(surface_name.as_str()) {
                continue;
            }
            let selected = self.active == Some(note.meta.id);
            let card = widgets::note_card(note, id, selected, &self.config, &self.palette);
            stack = stack.push(pin(card).x(note.meta.x).y(note.meta.y));
        }

        stack.into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            iced::output_events().map(Message::OutputEvent),
            store::subscription(self.config.notes_path()).map(|()| Message::NotesChanged),
            config::subscription(&self.config_path).map(|()| Message::ConfigChanged),
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

    fn target_output(&self, note: &Note) -> Option<String> {
        let primary = self.outputs.primary_name()?.to_owned();
        match note.meta.output.as_deref() {
            Some(name) if self.outputs.has_name(name) => Some(name.to_owned()),
            _ => Some(primary),
        }
    }

    /// The output name of the surface backing the given window id, if it is one
    /// of ours.
    fn output_name_for_window(&self, window: window::Id) -> Option<String> {
        self.outputs
            .entries
            .iter()
            .find(|entry| window::Id::from(entry.surface_id) == window)
            .map(|entry| entry.name.clone())
    }

    /// The logical size of the output backing the given window id, if known.
    fn output_size_for_window(&self, window: window::Id) -> Option<(f32, f32)> {
        self.outputs
            .entries
            .iter()
            .find(|entry| window::Id::from(entry.surface_id) == window)
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
        }
        self.active = Some(id);
        tasks.push(focus_editor(widgets::editor_id(id)));
        Task::batch(tasks)
    }

    fn exit_editing(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
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
                    return self.save_task(drag.note);
                }
            }
            if self.resize.is_some_and(|resize| resize.window != window) {
                self.retarget_resize(window, point);
            }
        }

        if let Some(drag) = self
            .drag
            .filter(|drag| drag.window == window && !drag.released)
        {
            let bounds = self.output_bounds_for_note(drag.note);
            if let Some(note) = self.find_note_mut(drag.note) {
                let mut x = drag.start_pos.x + (point.x - drag.start_cursor.x);
                let mut y = drag.start_pos.y + (point.y - drag.start_cursor.y);
                match bounds {
                    Some((ow, oh)) => {
                        x = x.clamp(0.0, (ow - note.meta.width).max(0.0));
                        y = y.clamp(0.0, (oh - note.meta.height).max(0.0));
                    }
                    None => {
                        x = x.max(0.0);
                        y = y.max(0.0);
                    }
                }
                note.meta.x = x;
                note.meta.y = y;
            }
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

    fn cursor_entered(&mut self, window: window::Id) {
        self.pointer_over = Some(window);

        // Entering any of our surfaces keeps a cross-monitor drag alive.
        if self.output_name_for_window(window).is_some()
            && (self.drag.is_some() || self.resize.is_some())
        {
            self.pending_leave = None;
        }
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

        let bounds = self.output_size_for_window(window);
        if let Some(name) = self.output_name_for_window(window)
            && let Some(note) = self.find_note_mut(drag.note)
        {
            note.meta.output = Some(name);
            let mut x = start_pos.x.max(0.0);
            let mut y = start_pos.y.max(0.0);
            if let Some((ow, oh)) = bounds {
                x = x.clamp(0.0, (ow - note.meta.width).max(0.0));
                y = y.clamp(0.0, (oh - note.meta.height).max(0.0));
            }
            note.meta.x = x;
            note.meta.y = y;
        }

        self.drag = Some(Drag {
            window,
            note: drag.note,
            start_cursor: point,
            start_pos,
            released: drag.released,
        });
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
        // Releasing on a different surface transfers the drag there first.
        if self.output_name_for_window(window).is_some() {
            if self.drag.is_some_and(|drag| drag.window != window) {
                let point = self.cursor.get(&window).copied();
                if let Some(point) = point {
                    self.retarget_drag(window, point);
                } else if let Some(name) = self.output_name_for_window(window)
                    && let Some(drag) = self.drag
                    && let Some(note) = self.find_note_mut(drag.note)
                {
                    note.meta.output = Some(name);
                }
            }
            if self.resize.is_some_and(|resize| resize.window != window)
                && let Some(name) = self.output_name_for_window(window)
                && let Some(resize) = self.resize
                && let Some(note) = self.find_note_mut(resize.note)
            {
                note.meta.output = Some(name);
            }
        }

        if let Some(drag) = self.drag.filter(|drag| drag.window == window) {
            // Wayland keeps an implicit pointer grab while a button is held, so
            // the pointer can be on another monitor while we only see
            // out-of-bounds coordinates. Keep the drag alive until the pointer
            // is picked up there (or the grace period expires).
            if self.pointer_outside_window(window) {
                self.drag = Some(Drag {
                    released: true,
                    ..drag
                });
                return self.start_leave_grace(window);
            }
            self.drag = None;
            self.pending_leave = None;
            return self.save_task(drag.note);
        }
        if let Some(resize) = self.resize.filter(|resize| resize.window == window) {
            self.resize = None;
            self.pending_leave = None;
            return self.save_task(resize.note);
        }
        Task::none()
    }

    /// The pointer left this surface, so it is now over another surface (a
    /// window, the bar, another monitor, ...). Leave editing mode, and give an
    /// in-flight drag a grace period to be picked up by another monitor before
    /// treating it as dropped.
    fn cursor_left(&mut self, window: window::Id) -> Task<Message> {
        if self.pointer_over == Some(window) {
            self.pointer_over = None;
        }

        let mut tasks = vec![self.exit_editing()];

        let dragging = self.drag.is_some_and(|drag| drag.window == window)
            || self.resize.is_some_and(|resize| resize.window == window);

        if dragging {
            tasks.push(self.start_leave_grace(window));
        }

        Task::batch(tasks)
    }

    fn confirm_leave(&mut self, token: u64) -> Task<Message> {
        let Some(pending) = self.pending_leave.filter(|pending| pending.token == token) else {
            return Task::none();
        };
        self.pending_leave = None;

        if let Some(drag) = self.drag.filter(|drag| drag.window == pending.window) {
            // A released drag that landed on another of our surfaces finishes
            // there, at the pointer's position.
            if drag.released
                && let Some(over) = self.pointer_over.filter(|over| *over != pending.window)
                && self.output_name_for_window(over).is_some()
            {
                if let Some(point) = self.cursor.get(&over).copied() {
                    self.retarget_drag(over, point);
                } else if let Some(name) = self.output_name_for_window(over)
                    && let Some(note) = self.find_note_mut(drag.note)
                {
                    note.meta.output = Some(name);
                }
                if let Some(drag) = self.drag.take() {
                    return self.save_task(drag.note);
                }
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
            return self.save_task(drag.note);
        }

        if let Some(resize) = self.resize.filter(|resize| resize.window == pending.window) {
            if let Some(over) = self.pointer_over
                && over != pending.window
                && self.output_name_for_window(over).is_some()
            {
                return Task::none();
            }
            self.resize = None;
            return self.save_task(resize.note);
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
                task
            }
            OutputEvent::InfoChanged(info) => {
                self.outputs.set_logical_size(info.id, info.logical_size);
                Task::none()
            }
            OutputEvent::Removed(id) => self.outputs.remove(id),
            OutputEvent::SurfaceEnteredOutput { .. } | OutputEvent::SurfaceLeftOutput { .. } => {
                Task::none()
            }
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
