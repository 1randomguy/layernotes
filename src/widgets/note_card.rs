use uuid::Uuid;

use iced::widget::{Space, Stack, container, markdown, mouse_area, scrollable, text, text_editor};
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
                    top_left: 8.0,
                    top_right: 8.0,
                    ..Default::default()
                },
                ..Border::default()
            },
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

    let card = container(iced::widget::column(vec![header.into(), body]).spacing(0))
        .width(Length::Fixed(note.meta.width))
        .height(Length::Fixed(note.meta.height))
        .padding(border_width)
        .style(move |_theme: &Theme| container::Style {
            background: Some(background.into()),
            border: Border {
                color: if selected { accent } else { border_color },
                width: border_width,
                radius: 10.0.into(),
            },
            shadow: Shadow {
                color: shadow_color,
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            text_color: Some(text_color),
            ..container::Style::default()
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

fn icon_button<'a>(label: &'a str, message: Message, color: Color) -> Element<'a, Message> {
    mouse_area(
        container(text(label).size(12).color(color)).padding(Padding {
            top: 2.0,
            right: 6.0,
            bottom: 2.0,
            left: 6.0,
        }),
    )
    .on_press(message)
    .interaction(mouse::Interaction::Pointer)
    .into()
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
