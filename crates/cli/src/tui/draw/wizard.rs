use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
};

use super::ThemeColors;

pub fn draw_wizard_dialog(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &str,
    step: usize,
    total_steps: usize,
    theme: &ThemeColors,
) {
    let popup_width = 60;
    let popup_height = 10;

    let popup = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    f.render_widget(Clear, popup);

    let progress_text = format!("Step {} of {}", step + 1, total_steps);
    let progress = (step as f64 / total_steps as f64) * popup_width as f64;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary))
        .title(format!("🧙 {} - {}", title, progress_text))
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(theme.surface));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(inner);

    let progress_gauge = Gauge::default()
        .gauge_style(Style::default().fg(theme.primary))
        .ratio(progress / popup_width as f64);
    f.render_widget(progress_gauge, layout[0]);

    let prompt_para = Paragraph::new(prompt)
        .style(Style::default().fg(theme.text))
        .wrap(Wrap { trim: true });
    f.render_widget(prompt_para, layout[1]);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title("Input");

    let input_para = Paragraph::new(format!("{}_", input))
        .style(Style::default().fg(theme.text))
        .block(input_block);
    f.render_widget(input_para, layout[2]);

    let help_text = "Enter: Confirm  |  Esc: Cancel  |  Backspace: Delete";
    let help_para = Paragraph::new(help_text)
        .style(Style::default().fg(theme.muted))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(help_para, layout[3]);
}
