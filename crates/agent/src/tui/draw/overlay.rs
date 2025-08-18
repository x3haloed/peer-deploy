use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use super::ThemeColors;

pub fn draw_overlay(f: &mut ratatui::Frame<'_>, area: Rect, text: &str, theme: &ThemeColors) {
    let text_width = text.len().min(60) + 4;
    let text_lines = (text.len() / 50) + 1;
    let popup_height = (text_lines + 2).min(8);

    let popup = Rect {
        x: area.x + (area.width.saturating_sub(text_width as u16)) / 2,
        y: area.y + area.height / 3,
        width: text_width as u16,
        height: popup_height as u16,
    };

    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(theme.primary))
        .style(Style::default().bg(theme.surface));

    let content = Paragraph::new(text)
        .style(Style::default().fg(theme.text))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(block);

    f.render_widget(content, popup);
}
