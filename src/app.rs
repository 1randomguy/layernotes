use std::collections::HashMap;
use std::path::PathBuf;

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

#[derive(Debug, Clone, Copy)]
struct Drag {
    window: window::Id,
    note: Uuid,
    start_cursor: Point,
    start_pos: Point,
}

#[derive(Debug, Clone, Copy)]
struct Resize {
    window: window::Id,
    note: Uuid,
    start_cursor: Point,
    start_size: Size,
}

#[derive(Debug, Clone)]
pub enum Message {
    OutputEvent(OutputEvent),
    ConfigChanged,
    NotesChanged,
    EscapePressed,
    BackgroundPressed(SurfaceId),
    NewNote(SurfaceId),
    Select(SurfaceId, Uuid),
    ToggleEdit(SurfaceId, Uuid),
    DeleteNote(Uuid),
    DragStart(SurfaceId, Uuid),
    ResizeStart(SurfaceId, Uuid),
    EditorAction(SurfaceId, Uuid, text_editor::Action),
    LinkClicked(String),
    CursorMoved(window::Id, Point),
    CursorReleased(window::Id),
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
    save_handles: HashMap<Uuid, iced::task::Handle>,
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

        let palette = Palette::from_config(&config.theme);
        let outputs = Outputs::new(config.layer);

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
                save_handles: HashMap::new(),
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
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
            Message::EscapePressed | Message::BackgroundPressed(_) => self.exit_editing(),
            Message::NewNote(surface) => self.new_note(surface),
            Message::Select(_, id) => self.select(id),
            Message::ToggleEdit(_, id) => self.toggle_edit(id),
            Message::DeleteNote(id) => self.delete_note(id),
            Message::DragStart(surface, id) => {
                self.drag_start(surface, id);
                Task::none()
            }
            Message::ResizeStart(surface, id) => {
                self.resize_start(surface, id);
                Task::none()
            }
            Message::EditorAction(_, id, action) => self.editor_action(id, action),
            Message::LinkClicked(url) => {
                self.open_link(&url);
                Task::none()
            }
            Message::CursorMoved(window, point) => {
                self.cursor_moved(window, point);
                Task::none()
            }
            Message::CursorReleased(window) => self.cursor_released(window),
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
            return Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        };
        let surface_name = entry.name.clone();
        let logical_size = entry.logical_size;

        let mut stack = Stack::new().push(
            mouse_area(
                Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::BackgroundPressed(id))
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

        if self.config.show_new_button
            && let Some((width, height)) = logical_size
        {
            stack = stack.push(
                pin(widgets::new_note_button(id, &self.palette))
                    .x((width as f32) - 60.0)
                    .y((height as f32) - 60.0),
            );
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
                iced::event::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::CursorReleased(window))
                }
                iced::event::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                    match key {
                        keyboard::Key::Named(keyboard::key::Named::Escape) => {
                            Some(Message::EscapePressed)
                        }
                        _ => None,
                    }
                }
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

    fn cursor_moved(&mut self, window: window::Id, point: Point) {
        self.cursor.insert(window, point);

        if let Some(drag) = self
            .drag
            .as_ref()
            .filter(|drag| drag.window == window)
            .copied()
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

        if let Some(resize) = self
            .resize
            .as_ref()
            .filter(|resize| resize.window == window)
            .copied()
        {
            let bounds = self.output_bounds_for_note(resize.note);
            if let Some(note) = self.find_note_mut(resize.note) {
                let mut width = (resize.start_size.width + (point.x - resize.start_cursor.x)).max(140.0);
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
    }

    fn cursor_released(&mut self, window: window::Id) -> Task<Message> {
        if let Some(drag) = self
            .drag
            .as_ref()
            .filter(|drag| drag.window == window)
            .copied()
        {
            self.drag = None;
            return self.save_task(drag.note);
        }
        if let Some(resize) = self
            .resize
            .as_ref()
            .filter(|resize| resize.window == window)
            .copied()
        {
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
        let mut by_id: HashMap<Uuid, Note> =
            loaded.into_iter().map(|note| (note.meta.id, note)).collect();
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
                let notes_dir_changed = new_config.notes_path() != self.config.notes_path();
                let layer_changed = new_config.layer != self.config.layer;
                self.config = new_config;
                self.palette = Palette::from_config(&self.config.theme);

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
                log::warn!("Failed to reload config: {e}");
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
