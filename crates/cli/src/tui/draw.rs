use ratatui::style::Color;

// Color scheme constants for consistent theming
pub const THEME_PRIMARY: Color = Color::Rgb(79, 172, 254); // Blue
pub const THEME_SUCCESS: Color = Color::Rgb(34, 197, 94); // Green
pub const THEME_WARNING: Color = Color::Rgb(251, 191, 36); // Yellow
pub const THEME_ERROR: Color = Color::Rgb(239, 68, 68); // Red
pub const THEME_MUTED: Color = Color::Rgb(156, 163, 175); // Gray
pub const THEME_ACCENT: Color = Color::Rgb(168, 85, 247); // Purple
pub const THEME_BACKGROUND: Color = Color::Rgb(17, 24, 39); // Dark blue-gray
pub const THEME_SURFACE: Color = Color::Rgb(31, 41, 55); // Lighter blue-gray
pub const THEME_TEXT: Color = Color::Rgb(243, 244, 246); // Light gray

mod footer;
mod header;
mod logs;
mod overlay;
mod overview;
mod peers;
mod placeholder;
mod topology;
mod wizard;

pub use footer::draw_footer;
pub use header::draw_header_tabs;
pub use logs::{draw_component_logs, draw_logs};
pub use overlay::draw_overlay;
pub use overview::draw_overview;
pub use peers::draw_peers;
pub use placeholder::draw_placeholder;
pub use topology::draw_topology;
#[allow(unused_imports)]
pub use wizard::draw_wizard_dialog;
