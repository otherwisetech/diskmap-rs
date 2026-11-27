//! The family's tokens come from otherwise-ui (github.com/otherwisetech/otherwise-ui, by tag):
//! Inter, the type scale, the warm neutrals and status colours, with diskmap's green accent and
//! blue badge (`otherwise_ui::accents::DISKMAP`, set in main.rs); plus one colour per file kind.
use gpui::{App, Hsla, rgb};
use scan::Kind;

/// The family's colours (`otherwise_ui::Theme`, through `Deref`); the file-kind fills below
/// live inside the treemap only.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub ui: otherwise_ui::Theme,
}

impl std::ops::Deref for Tokens {
    type Target = otherwise_ui::Theme;
    fn deref(&self) -> &otherwise_ui::Theme {
        &self.ui
    }
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Tokens {
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
    Tokens { ui: otherwise_ui::theme(cx) }
}

pub use otherwise_ui::tokens::SANS;
