use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use super::ThemeColors;

pub fn draw_placeholder(f: &mut ratatui::Frame<'_>, area: Rect, text: &str, theme: &ThemeColors) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(5),
            Constraint::Percentage(40),
        ])
        .split(area);

    let center_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .split(layout[1]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.muted))
        .title("🚧 Coming Soon")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));

    let content = Paragraph::new(text)
        .style(Style::default().fg(theme.muted))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(block);

    f.render_widget(content, center_layout[1]);
}
