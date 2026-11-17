use ratatui::style::{Color, Modifier, Style};

pub const BLUE: (u8, u8, u8) = (0xae, 0xbd, 0xe8);
pub const CORAL: (u8, u8, u8) = (0xe3, 0x8c, 0x9a);
pub const GOLD: (u8, u8, u8) = (0xe4, 0xbf, 0x79);
pub const GREEN: (u8, u8, u8) = (0x9c, 0xcf, 0xa0);
pub const DIM: (u8, u8, u8) = (0x99, 0x93, 0xa5);

#[derive(Clone, Copy)]
pub enum Theme {
    True,
    Ansi,
    Mono,
}

impl Theme {
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::Mono;
        }
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|value| value.contains("truecolor") || value.contains("24bit"));
        if truecolor { Self::True } else { Self::Ansi }
    }

    pub fn fg(self, color: (u8, u8, u8)) -> Style {
        match self {
            Self::True => Style::new().fg(Color::Rgb(color.0, color.1, color.2)),
            Self::Ansi => Style::new().fg(nearest_ansi(color)),
            Self::Mono => Style::new(),
        }
    }

    pub fn dim(self, color: (u8, u8, u8)) -> Style {
        match self {
            Self::Mono => Style::new().add_modifier(Modifier::DIM),
            _ => self.fg(color),
        }
    }

    pub fn selection(self) -> Style {
        match self {
            Self::True => Style::new().bg(Color::Rgb(0x24, 0x27, 0x33)),
            Self::Ansi | Self::Mono => Style::new().add_modifier(Modifier::REVERSED),
        }
    }
}

fn nearest_ansi(color: (u8, u8, u8)) -> Color {
    let (r, g, b) = color;
    if r > 180 && g > 150 && b < 160 {
        Color::Yellow
    } else if g > r && g >= b {
        Color::Green
    } else if b >= r && b >= g {
        Color::Cyan
    } else if r > g && b > g {
        Color::Magenta
    } else {
        Color::Gray
    }
}
