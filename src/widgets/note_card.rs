use uuid::Uuid;

use iced::widget::{
    Space, Stack, container, markdown, mouse_area, scrollable, svg, text, text_editor,
};
use iced::{
    Border, Color, Element, Length, Padding, Shadow, SurfaceId, Theme, Vector, alignment, mouse,
};

use crate::app::Message;
use crate::config::Config;
use crate::note::Note;
use crate::theme::{self, Palette};

pub fn editor_id(id: Uuid) -> iced::widget::Id {
    iced::widget::Id::from(format!("note-editor-{id}"))
}

pub fn note_card<'a>(
    note: &'a Note,
    surface: SurfaceId,
    selected: bool,
    config: &Config,
    palette: &Palette,
) -> Element<'a, Message> {
    let id = note.meta.id;
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
    let font_size = note.meta.font_size.unwrap_or(config.font_size);
    let accent = palette.accent;
    let border_color = palette.border;
    let shadow_color = palette.shadow;
    // The header follows the note's own colour rather than the theme, so a
    // yellow note keeps a yellow (slightly shaded) bar with dark text.
    let header_bg = theme::shade(background);
    let border_width = 2.0;
    let radius = 10.0;
    let inner_radius = radius - border_width;

    let header = mouse_area(
        container(
            iced::widget::row(vec![
                text(note.display_title())
                    .size((font_size - 2.0).max(10.0))
                    .color(text_color)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::Word)
                    .into(),
                icon_button(
                    pin_icon(if note.pinned { accent } else { text_color }, note.pinned),
                    Message::TogglePin(id),
                ),
                text_button(
                    if note.confirm_delete { "sure?" } else { "x" },
                    Message::DeleteNote(id),
                    if note.confirm_delete {
                        accent
                    } else {
                        text_color
                    },
                ),
            ])
            .align_y(alignment::Vertical::Center)
            .spacing(6),
        )
        .padding([4, 8])
        .style(move |_theme: &Theme| container::Style {
            background: Some(header_bg.into()),
            border: Border {
                radius: iced::border::Radius {
                    top_left: inner_radius,
                    top_right: inner_radius,
                    ..Default::default()
                },
                ..Border::default()
            },
            // Keep the header edge on the pixel grid so it lines up with the
            // card border instead of leaving a sub-pixel seam.
            snap: true,
            ..container::Style::default()
        }),
    )
    .on_press(Message::DragStart(surface, id))
    .interaction(mouse::Interaction::Grab);

    let body: Element<'a, Message> = if note.editing {
        match &note.editor {
            Some(content) => text_editor(content)
                .id(editor_id(id))
                .on_action(move |action| Message::EditorAction(id, action))
                .padding(10)
                .size(font_size)
                .style(move |_theme: &Theme, _status| text_editor::Style {
                    background: Color::TRANSPARENT.into(),
                    border: Border::default(),
                    placeholder: text_color.scale_alpha(0.45),
                    value: text_color,
                    selection: accent.scale_alpha(0.4),
                })
                .highlight("markdown", iced::highlighter::Theme::Base16Ocean)
                .height(Length::Fill)
                .into(),
            None => Space::new().width(Length::Fill).height(Length::Fill).into(),
        }
    } else {
        scrollable(
            container(
                markdown::view(
                    &note.items,
                    markdown_settings(font_size, text_color, palette, &config.markdown),
                )
                .map(Message::LinkClicked),
            )
            .padding(10)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    };

    // The note background is its own rounded panel rather than a `border` on
    // the card. iced draws a border as an antialiased stroke over the same
    // quad as the background; at fractional note positions its curved edges
    // blend with the background and leave a hairline around the rounded
    // corners. Two nested solid quads (ring + panel) only ever blend opaque
    // colours with each other, so no background can bleed through.
    let panel = container(iced::widget::column(vec![header.into(), body]).spacing(0))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(background.into()),
            border: Border {
                radius: inner_radius.into(),
                ..Border::default()
            },
            text_color: Some(text_color),
            // Keep the panel on the pixel grid so it lines up with the ring.
            snap: true,
            ..container::Style::default()
        });

    let ring_color = if selected { accent } else { border_color };

    let card = container(panel)
        .width(Length::Fixed(note.meta.width))
        .height(Length::Fixed(note.meta.height))
        .padding(border_width)
        .style(move |_theme: &Theme| container::Style {
            background: Some(ring_color.into()),
            border: Border {
                radius: radius.into(),
                ..Border::default()
            },
            shadow: Shadow {
                color: shadow_color,
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            text_color: Some(text_color),
            snap: true,
        });

    let resize_handle = mouse_area(
        container(
            Space::new()
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0)),
        )
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0)),
    )
    .on_press(Message::ResizeStart(surface, id))
    .interaction(mouse::Interaction::ResizingDiagonallyDown);

    let resize_overlay = container(resize_handle)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .align_y(alignment::Vertical::Bottom);

    mouse_area(Stack::new().push(card).push(resize_overlay))
        .on_press(Message::Select(id))
        .on_double_click(Message::ToggleEdit(id))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// A pushpin symbol, drawn as a monochrome SVG so it does not depend on a
/// font having the glyph.
const PIN_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"/></svg>"#;

/// The pin toggle. Upright and tinted with the accent when pinned, tilted and
/// muted otherwise.
fn pin_icon<'a>(color: Color, pinned: bool) -> Element<'a, Message> {
    let rotation = if pinned {
        0.0
    } else {
        std::f32::consts::FRAC_PI_4
    };

    svg(svg::Handle::from_memory(PIN_SVG))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .rotation(rotation)
        .style(move |_theme: &Theme, _status| svg::Style { color: Some(color) })
        .into()
}

fn icon_button<'a>(content: Element<'a, Message>, message: Message) -> Element<'a, Message> {
    mouse_area(container(content).padding(Padding {
        top: 2.0,
        right: 6.0,
        bottom: 2.0,
        left: 6.0,
    }))
    .on_press(message)
    .interaction(mouse::Interaction::Pointer)
    .into()
}

fn text_button<'a>(label: &'a str, message: Message, color: Color) -> Element<'a, Message> {
    icon_button(text(label).size(12).color(color).into(), message)
}

fn markdown_settings(
    font_size: f32,
    text_color: Color,
    palette: &Palette,
    markdown_config: &crate::config::MarkdownConfig,
) -> markdown::Settings {
    let mut style = markdown::Style::from_palette(Theme::CatppuccinMocha.palette());
    style.link_color = palette.accent;
    style.inline_code_color = text_color;
    style.inline_code_highlight = iced::core::text::Highlight {
        background: text_color.scale_alpha(0.12).into(),
        border: Border::default().rounded(4.0),
    };

    markdown::Settings {
        text_size: font_size.into(),
        h1_size: (font_size * markdown_config.h1_scale).into(),
        h2_size: (font_size * markdown_config.h2_scale).into(),
        h3_size: (font_size * markdown_config.h3_scale).into(),
        h4_size: (font_size * markdown_config.h4_scale).into(),
        h5_size: (font_size * markdown_config.h5_scale).into(),
        h6_size: (font_size * markdown_config.h6_scale).into(),
        code_size: (font_size * markdown_config.code_scale).into(),
        spacing: (font_size * markdown_config.spacing_scale).into(),
        style,
    }
}
