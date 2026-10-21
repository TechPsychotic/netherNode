//! netherNode's look: obsidian and violet glass over a live nether portal.
//!
//! "Glass" panels are translucent violet-tinted surfaces with a light rim and a soft
//! shadow, so the animated portal glows through them. (iced has no backdrop blur, so the
//! tint is kept fairly dense to keep text readable.)

use iced::theme::Palette;
use iced::widget::{button, container, pick_list, progress_bar, scrollable, text_input};
use iced::{Background, Border, Color, Gradient, Radians, Shadow, Theme, Vector, gradient};

const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

const fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

pub const OBSIDIAN: Color = rgb(0x0c0814);
pub const GLASS: Color = Color::from_rgba(0.075, 0.045, 0.125, 0.70);
pub const GLASS_DENSE: Color = Color::from_rgba(0.055, 0.032, 0.095, 0.86);
pub const RIM: Color = Color::from_rgba(0.78, 0.62, 1.0, 0.22);
pub const TEXT: Color = rgb(0xf3edff);
pub const MUTED: Color = rgb(0xb4a7cf);
pub const VIOLET: Color = rgb(0x9b5cff);
pub const VIOLET_BRIGHT: Color = rgb(0xc8a2ff);
pub const MAGENTA: Color = rgb(0xd65cff);
pub const SUCCESS: Color = rgb(0x5ae3a8);
pub const WARNING: Color = rgb(0xffc861);
pub const DANGER: Color = rgb(0xff5d7e);

pub fn theme() -> Theme {
    Theme::custom(
        "netherNode Obsidian",
        Palette {
            background: OBSIDIAN,
            text: TEXT,
            primary: VIOLET,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

fn rim(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

fn glow(color: Color, blur: f32) -> Shadow {
    Shadow {
        color,
        offset: Vector::new(0.0, 6.0),
        blur_radius: blur,
    }
}

fn violet_gradient(a: f32) -> Background {
    Background::Gradient(Gradient::Linear(
        gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_2 * 0.6))
            .add_stop(0.0, alpha(rgb(0x7b3dff), a))
            .add_stop(1.0, alpha(MAGENTA, a)),
    ))
}

/// Used instead of the portal when shaders aren't available (CPU renderer).
pub fn static_backdrop(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            gradient::Linear::new(Radians(std::f32::consts::PI * 0.75))
                .add_stop(0.0, rgb(0x2a0f4f))
                .add_stop(0.55, rgb(0x120824))
                .add_stop(1.0, rgb(0x050309)),
        ))),
        ..container::Style::default()
    }
}

pub fn glass(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(GLASS)),
        border: rim(RIM, 18.0),
        shadow: glow(Color::from_rgba(0.0, 0.0, 0.0, 0.45), 28.0),
        text_color: Some(TEXT),
        snap: true,
    }
}

pub fn glass_dense(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(GLASS_DENSE)),
        border: rim(RIM, 18.0),
        shadow: glow(Color::from_rgba(0.0, 0.0, 0.0, 0.5), 32.0),
        text_color: Some(TEXT),
        snap: true,
    }
}

pub fn console(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.02, 0.01, 0.04, 0.82))),
        border: rim(alpha(RIM, 0.14), 12.0),
        text_color: Some(rgb(0xd8cfe8)),
        ..container::Style::default()
    }
}

pub fn row_item(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.04))),
        border: rim(alpha(RIM, 0.12), 12.0),
        ..container::Style::default()
    }
}

pub fn notice(is_error: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let color = if is_error { DANGER } else { SUCCESS };
        container::Style {
            background: Some(Background::Color(Color { a: 0.16, ..color })),
            border: rim(Color { a: 0.55, ..color }, 12.0),
            text_color: Some(TEXT),
            ..container::Style::default()
        }
    }
}

/// Small rounded label, e.g. the loader name on a card.
pub fn chip(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(alpha(color, 0.18))),
        border: rim(alpha(color, 0.5), 999.0),
        text_color: Some(color),
        ..container::Style::default()
    }
}

/// Instance cards in the grid.
pub fn card(selected: bool, hovered: bool) -> container::Style {
    let background = match (selected, hovered) {
        (true, _) => Background::Color(Color::from_rgba(0.38, 0.2, 0.7, 0.32)),
        (false, true) => Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)),
        _ => Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.04)),
    };
    container::Style {
        background: Some(background),
        border: rim(
            if selected {
                VIOLET_BRIGHT
            } else {
                alpha(RIM, 0.16)
            },
            14.0,
        ),
        shadow: if selected {
            glow(alpha(VIOLET, 0.55), 22.0)
        } else {
            Shadow::default()
        },
        text_color: Some(TEXT),
        snap: true,
    }
}

pub fn play(_: &Theme, status: button::Status) -> button::Style {
    let (background, shadow) = match status {
        button::Status::Disabled => (
            Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)),
            Shadow::default(),
        ),
        button::Status::Hovered => (violet_gradient(1.0), glow(alpha(MAGENTA, 0.7), 30.0)),
        button::Status::Pressed => (violet_gradient(0.8), glow(alpha(MAGENTA, 0.4), 14.0)),
        button::Status::Active => (violet_gradient(0.95), glow(alpha(VIOLET, 0.55), 22.0)),
    };
    button::Style {
        background: Some(background),
        text_color: if status == button::Status::Disabled {
            MUTED
        } else {
            Color::WHITE
        },
        border: rim(alpha(Color::WHITE, 0.25), 14.0),
        shadow,
        snap: true,
    }
}

pub fn stop(_: &Theme, status: button::Status) -> button::Style {
    let a = if status == button::Status::Hovered {
        1.0
    } else {
        0.85
    };
    button::Style {
        background: Some(Background::Color(alpha(DANGER, a))),
        text_color: Color::WHITE,
        border: rim(alpha(Color::WHITE, 0.2), 14.0),
        shadow: glow(alpha(DANGER, 0.45), 20.0),
        snap: true,
    }
}

pub fn glass_button(_: &Theme, status: button::Status) -> button::Style {
    let a = match status {
        button::Status::Hovered => 0.14,
        button::Status::Pressed => 0.2,
        _ => 0.07,
    };
    button::Style {
        background: Some(Background::Color(Color::from_rgba(0.85, 0.75, 1.0, a))),
        text_color: if status == button::Status::Disabled {
            alpha(MUTED, 0.5)
        } else {
            TEXT
        },
        border: rim(
            alpha(
                RIM,
                if status == button::Status::Hovered {
                    0.5
                } else {
                    0.25
                },
            ),
            10.0,
        ),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(alpha(
            DANGER,
            if hovered { 0.22 } else { 0.08 },
        ))),
        text_color: DANGER,
        border: rim(alpha(DANGER, 0.55), 10.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Tabs and segmented choices (loader picker).
pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if selected {
            button::Style {
                shadow: glow(alpha(VIOLET, 0.4), 14.0),
                ..play(theme, button::Status::Active)
            }
        } else {
            glass_button(theme, status)
        }
    }
}

pub fn link(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: if status == button::Status::Hovered {
            Color::WHITE
        } else {
            VIOLET_BRIGHT
        },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(Color::from_rgba(0.02, 0.01, 0.05, 0.6)),
        border: rim(
            if focused {
                VIOLET_BRIGHT
            } else {
                alpha(RIM, 0.3)
            },
            10.0,
        ),
        icon: MUTED,
        placeholder: alpha(MUTED, 0.7),
        value: TEXT,
        selection: alpha(VIOLET, 0.45),
    }
}

pub fn picker(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let hovered = !matches!(status, pick_list::Status::Active);
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: MUTED,
        handle_color: VIOLET_BRIGHT,
        background: Background::Color(Color::from_rgba(0.02, 0.01, 0.05, 0.6)),
        border: rim(alpha(RIM, if hovered { 0.55 } else { 0.3 }), 10.0),
    }
}

pub fn progress(_: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)),
        bar: violet_gradient(1.0),
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
    }
}

pub fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = Background::Color(alpha(VIOLET_BRIGHT, 0.35));
        rail.scroller.border = Border {
            radius: 999.0.into(),
            ..Border::default()
        };
    }
    style
}
