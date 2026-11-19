//! Design tokens (same palette family as genealogy-rs) plus one colour per file kind.
use gpui::{App, Hsla, rgb};
use gpui_component::ActiveTheme;
use scan::Kind;

#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub ink: Hsla,
    pub ink_muted: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    /// logo badge background (the icon's blue)
    pub brand: Hsla,
    pub dark: bool,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Tokens {
    pub fn light() -> Self {
        Tokens {
            canvas: c(0xEFEDE8),
            surface: c(0xFFFFFF),
            border: c(0xE2DFD8),
            border_strong: c(0xC9C5BB),
            ink: c(0x1C1B19),
            ink_muted: c(0x6B675F),
            accent: c(0x2F6B4F),
            accent_soft: c(0xE6F0EA),
            danger: c(0xA3382A),
            warning: c(0xA86A12),
            brand: c(0x3F628F),
            dark: false,
        }
    }

    pub fn dark() -> Self {
        let mut accent_soft = c(0x2F6B4F);
        accent_soft.a = 0.25;
        Tokens {
            canvas: c(0x141413),
            surface: c(0x1F1E1C),
            border: c(0x34322E),
            border_strong: c(0x4A4741),
            ink: c(0xF2F0EA),
            ink_muted: c(0xA8A39A),
            accent: c(0x4F9A75),
            accent_soft,
            danger: c(0xD0604F),
            warning: c(0xD9A441),
            brand: c(0x4A6E9E),
            dark: true,
        }
    }

    /// text on accent/ink filled controls
    pub fn on_fill(&self) -> Hsla {
        if self.dark { c(0x141413) } else { c(0xFFFFFF) }
    }

    /// Fill of a file tile.
    pub fn kind(&self, k: Kind) -> Hsla {
        let hex = match k {
            Kind::Video => 0xC2553D,
            Kind::Image => 0x3E8E7E,
            Kind::Audio => 0x8E5BB5,
            Kind::Archive => 0xC58B2A,
            Kind::Code => 0x3D6FB6,
            Kind::Document => 0x6B8E3D,
            Kind::Package => 0xB5577F,
            Kind::Other | Kind::Dir => 0x8F8A80,
        };
        let mut h = c(hex);
        if self.dark {
            h.l = (h.l * 0.85).min(0.55);
            h.s *= 0.85;
        }
        h
    }

    /// Fill of a directory tile: a little deeper with every nesting level.
    pub fn dir(&self, depth: u16) -> Hsla {
        let d = depth.min(8) as f32;
        let mut h = if self.dark { c(0x2A2926) } else { c(0xE6E3DC) };
        h.l += if self.dark { d * 0.018 } else { -d * 0.022 };
        h
    }

    /// Text on a file tile.
    pub fn on_kind(&self) -> Hsla {
        c(0xFFFFFF).opacity(0.95)
    }
}

pub fn tokens(cx: &App) -> Tokens {
    if cx.theme().mode.is_dark() { Tokens::dark() } else { Tokens::light() }
}

pub const SANS: &str = "Inter";
