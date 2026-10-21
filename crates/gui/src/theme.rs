//! netherNode's look: a dark, warm palette with a nether-orange accent.

use iced::theme::Palette;
use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Theme};

const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

pub const BACKGROUND: Color = rgb(0x17151a);
pub const SURFACE: Color = rgb(0x211e25);
pub const SURFACE_RAISED: Color = rgb(0x2b2730);
pub const BORDER: Color = rgb(0x3a3540);
pub const TEXT: Color = rgb(0xece8ef);
pub const MUTED: Color = rgb(0x9b94a3);
pub const ACCENT: Color = rgb(0xe8613c);
pub const SUCCESS: Color = rgb(0x52b788);
pub const WARNING: Color = rgb(0xe5a93b);
pub const DANGER: Color = rgb(0xe0525e);
pub const CONSOLE: Color = rgb(0x0f0e11);

pub fn theme() -> Theme {
    Theme::custom(
        "netherNode",
        Palette {
            background: BACKGROUND,
            text: TEXT,
            primary: ACCENT,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

fn border(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: border(BORDER, 10.0),
        ..container::Style::default()
    }
}

pub fn console(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(CONSOLE)),
        border: border(BORDER, 8.0),
        text_color: Some(rgb(0xc9c3cf)),
        ..container::Style::default()
    }
}

pub fn notice(is_error: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let color = if is_error { DANGER } else { SUCCESS };
        container::Style {
            background: Some(Background::Color(Color { a: 0.12, ..color })),
            border: border(Color { a: 0.5, ..color }, 8.0),
            text_color: Some(TEXT),
            ..container::Style::default()
        }
    }
}

/// Instance entries in the sidebar.
pub fn list_item(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (selected, status) {
            (true, _) => Some(Background::Color(SURFACE_RAISED)),
            (false, button::Status::Hovered) => Some(Background::Color(Color {
                a: 0.5,
                ..SURFACE_RAISED
            })),
            _ => None,
        };
        button::Style {
            background,
            text_color: TEXT,
            border: Border {
                color: if selected { ACCENT } else { Color::TRANSPARENT },
                width: if selected { 1.0 } else { 0.0 },
                radius: 8.0.into(),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

fn filled(color: Color, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color { a: 0.85, ..color },
        button::Status::Pressed => Color { a: 0.7, ..color },
        button::Status::Disabled => Color { a: 0.35, ..color },
        button::Status::Active => color,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            MUTED
        } else {
            Color::WHITE
        },
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn play(_: &Theme, status: button::Status) -> button::Style {
    filled(ACCENT, status)
}

pub fn stop(_: &Theme, status: button::Status) -> button::Style {
    filled(DANGER, status)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => BORDER,
        _ => SURFACE_RAISED,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            MUTED
        } else {
            TEXT
        },
        border: border(BORDER, 8.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn danger_outline(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(Background::Color(Color { a: 0.15, ..DANGER })),
        text_color: DANGER,
        border: border(Color { a: 0.6, ..DANGER }, 8.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let base = text_input::default(theme, status);
    text_input::Style {
        background: Background::Color(BACKGROUND),
        border: border(
            if matches!(status, text_input::Status::Focused { .. }) {
                ACCENT
            } else {
                BORDER
            },
            8.0,
        ),
        ..base
    }
}
