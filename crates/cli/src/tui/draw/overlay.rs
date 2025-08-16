use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use super::{THEME_PRIMARY, THEME_SURFACE, THEME_TEXT};

pub fn draw_overlay(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
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
        .border_style(Style::default().fg(THEME_PRIMARY))
        .style(Style::default().bg(THEME_SURFACE));

    let content = Paragraph::new(text)
        .style(Style::default().fg(THEME_TEXT))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(block);

    f.render_widget(content, popup);
}
