use iced::widget::text::Wrapping;
use iced::widget::{Space, Stack, column, container, mouse_area, row, scrollable, text};
use iced::{Border, Element, Length, Padding, Shadow, SurfaceId, Theme, Vector, alignment, mouse};

use crate::app::Message;
use crate::config::Config;
use crate::note::Note;
use crate::theme::{self, Palette};

const CARD_WIDTH: f32 = 300.0;
const CARD_HEIGHT: f32 = 210.0;
const PREVIEW_LINES: usize = 4;

/// The small bottom-right button that opens the rescue board. Shows only a
/// count and an arrow (no label).
pub fn rescue_button<'a>(
    surface: SurfaceId,
    count: usize,
    palette: &Palette,
) -> Element<'a, Message> {
    let text_color = palette.text;
    let background = palette.note;
    let border_color = palette.border;
    let shadow = palette.shadow;

    mouse_area(
        container(text(format!("\u{2193} {count}")).size(14).color(text_color))
            .width(Length::Fixed(62.0))
            .height(Length::Fixed(40.0))
            .center_x(Length::Fixed(62.0))
            .center_y(Length::Fixed(40.0))
            .style(move |_theme: &Theme| container::Style {
                background: Some(background.into()),
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: 18.0.into(),
                },
                shadow: Shadow {
                    color: shadow,
                    offset: Vector::new(0.0, 2.0),
                    blur_radius: 8.0,
                },
                text_color: Some(text_color),
                ..container::Style::default()
            }),
    )
    .on_press(Message::OpenRescueBoard(surface))
    .interaction(mouse::Interaction::Pointer)
    .into()
}

/// The rescue board: notes whose monitor is disconnected, grouped by monitor.
pub fn rescue_board<'a>(
    groups: &[(String, Vec<&'a Note>)],
    palette: &Palette,
    config: &Config,
    surface_size: Option<(f32, f32)>,
) -> Element<'a, Message> {
    let (sw, sh) = surface_size.unwrap_or((1280.0, 800.0));
    let panel_width = (sw - 80.0).clamp(360.0, 1100.0);
    let panel_height = (sh - 80.0).clamp(280.0, 900.0);
    let columns = (((panel_width - 48.0 + 16.0) / (CARD_WIDTH + 16.0)).floor() as usize).max(1);

    let mut body = column(Vec::new()).spacing(18);
    for (name, notes) in groups {
        body = body.push(group_section(name, notes, palette, config, columns));
    }

    let panel_bg = palette.note;
    let panel_border = palette.border;
    let panel_shadow = palette.shadow;
    let panel_text = palette.text;

    let panel = container(
        column(vec![
            header(palette),
            scrollable(body)
                .height(Length::Fill)
                .width(Length::Fill)
                .into(),
        ])
        .spacing(14),
    )
    .width(Length::Fixed(panel_width))
    .height(Length::Fixed(panel_height))
    .padding(18)
    .style(move |_theme: &Theme| container::Style {
        background: Some(panel_bg.into()),
        border: Border {
            color: panel_border,
            width: 1.0,
            radius: 14.0.into(),
        },
        shadow: Shadow {
            color: panel_shadow,
            offset: Vector::new(0.0, 8.0),
            blur_radius: 28.0,
        },
        text_color: Some(panel_text),
        ..container::Style::default()
    });

    // Dim, click-to-close backdrop; the panel sits on top.
    Stack::new()
        .push(
            mouse_area(
                container(Space::new().width(Length::Fill).height(Length::Fill)).style(
                    |_theme: &Theme| container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
                        ..container::Style::default()
                    },
                ),
            )
            .on_press(Message::CloseRescueBoard),
        )
        .push(container(panel).center(Length::Fill))
        .into()
}

fn header<'a>(palette: &Palette) -> Element<'a, Message> {
    row(vec![
        text("Notes on disconnected monitors")
            .size(18)
            .color(palette.text)
            .width(Length::Fill)
            .into(),
        chip("\u{00d7}".to_owned(), Message::CloseRescueBoard, palette),
    ])
    .align_y(alignment::Vertical::Center)
    .into()
}

fn group_section<'a>(
    name: &str,
    notes: &[&'a Note],
    palette: &Palette,
    config: &Config,
    columns: usize,
) -> Element<'a, Message> {
    let title = row(vec![
        text(format!("{name}  ({})", notes.len()))
            .size(16)
            .color(palette.text)
            .width(Length::Fill)
            .into(),
        chip(
            "Move all here".to_owned(),
            Message::MoveAllHere(name.to_owned()),
            palette,
        ),
    ])
    .align_y(alignment::Vertical::Center)
    .spacing(8);

    let mut cards = column(Vec::new()).spacing(14);
    for chunk in notes.chunks(columns.max(1)) {
        let row = row(chunk
            .iter()
            .map(|note| board_card(note, palette, config))
            .collect::<Vec<_>>())
        .spacing(14);
        cards = cards.push(row);
    }

    column(vec![title.into(), cards.into()]).spacing(10).into()
}

fn board_card<'a>(note: &'a Note, palette: &Palette, config: &Config) -> Element<'a, Message> {
    let background = note
        .meta
        .color
        .as_deref()
        .and_then(theme::parse_hex)
        .or_else(|| config.default_color.as_deref().and_then(theme::parse_hex))
        .unwrap_or(palette.note);
    let text_color = if background == palette.note {
        palette.text
    } else {
        theme::contrast_text(background)
    };
    let font_size = config.font_size;
    let border_color = palette.border;
    let shadow = palette.shadow;

    let mut preview = column(Vec::new()).spacing(2);
    for line in note
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(PREVIEW_LINES)
    {
        preview = preview.push(
            text(truncate(line, 44))
                .size(font_size)
                .color(text_color.scale_alpha(0.85))
                .width(Length::Fill)
                .wrapping(Wrapping::Word),
        );
    }

    container(
        column(vec![
            text(note.display_title())
                .size(font_size + 3.0)
                .color(text_color)
                .width(Length::Fill)
                .wrapping(Wrapping::Word)
                .into(),
            preview.into(),
            Space::new().height(Length::Fill).into(),
            chip(
                "Move here".to_owned(),
                Message::MoveNoteHere(note.meta.id),
                palette,
            ),
        ])
        .spacing(8),
    )
    .width(Length::Fixed(CARD_WIDTH))
    .height(Length::Fixed(CARD_HEIGHT))
    .padding(14)
    .style(move |_theme: &Theme| container::Style {
        background: Some(background.into()),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow {
            color: shadow,
            offset: Vector::new(0.0, 2.0),
            blur_radius: 10.0,
        },
        text_color: Some(text_color),
        ..container::Style::default()
    })
    .into()
}

fn chip<'a>(label: String, message: Message, palette: &Palette) -> Element<'a, Message> {
    let text_color = palette.text;
    let accent = palette.accent;

    mouse_area(
        container(text(label).size(13).color(text_color))
            .padding(Padding {
                top: 4.0,
                right: 10.0,
                bottom: 4.0,
                left: 10.0,
            })
            .style(move |_theme: &Theme| container::Style {
                background: Some(accent.scale_alpha(0.18).into()),
                border: Border {
                    color: accent,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                text_color: Some(text_color),
                ..container::Style::default()
            }),
    )
    .on_press(message)
    .interaction(mouse::Interaction::Pointer)
    .into()
}

fn truncate(line: &str, max: usize) -> String {
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('\u{2026}');
    }
    out
}
