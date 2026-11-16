use nsh_core::prelude::*;

use crate::ui::card_text::Accent;

pub fn accent(accent: Accent) -> Rgba {
    match accent {
        Accent::Cyan => rgb(0x22d3ee),
        Accent::Magenta => rgb(0xe879f9),
        Accent::Yellow => rgb(0xfacc15),
        Accent::Green => rgb(0x4ade80),
        Accent::Red => rgb(0xf87171),
        Accent::Muted => muted(),
    }
}

pub fn muted() -> Rgba {
    rgb(0x737373)
}
