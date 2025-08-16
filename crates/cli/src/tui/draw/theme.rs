use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub primary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
    pub accent: Color,
    pub background: Color,
    pub surface: Color,
    pub text: Color,
}

pub fn get_theme(kind: ThemeKind) -> ThemeColors {
    match kind {
        ThemeKind::Dark => ThemeColors {
            // Tailwind-esque dark palette
            primary: Color::Rgb(79, 172, 254),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(251, 191, 36),
            error: Color::Rgb(239, 68, 68),
            muted: Color::Rgb(156, 163, 175),
            accent: Color::Rgb(168, 85, 247),
            background: Color::Rgb(17, 24, 39),  // slate-900
            surface: Color::Rgb(31, 41, 55),     // slate-800
            text: Color::Rgb(243, 244, 246),     // slate-100
        },
        ThemeKind::Light => ThemeColors {
            primary: Color::Rgb(37, 99, 235),   // blue-600
            success: Color::Rgb(22, 163, 74),   // green-600
            warning: Color::Rgb(245, 158, 11),  // amber-500
            error: Color::Rgb(220, 38, 38),     // red-600
            muted: Color::Rgb(100, 116, 139),   // slate-500
            accent: Color::Rgb(124, 58, 237),   // violet-600
            background: Color::Rgb(248, 250, 252), // slate-50
            surface: Color::Rgb(255, 255, 255),    // white
            text: Color::Rgb(15, 23, 42),       // slate-900
        },
    }
}


