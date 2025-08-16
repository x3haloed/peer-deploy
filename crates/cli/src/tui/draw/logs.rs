use std::collections::VecDeque;
use std::time::Instant;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState},
};

use super::{
    THEME_ACCENT, THEME_BACKGROUND, THEME_ERROR, THEME_MUTED, THEME_PRIMARY, THEME_TEXT,
    THEME_WARNING,
};

pub fn draw_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    events: &VecDeque<(Instant, String)>,
    filter: Option<&str>,
    paused: bool,
) {
    let filtered_events: Vec<_> = events
        .iter()
        .filter(|(_, s)| filter.map_or(true, |f| s.contains(f)))
        .collect();

    let items: Vec<ListItem> = filtered_events
        .iter()
        .enumerate()
        .map(|(i, (t, s))| {
            let elapsed = t.elapsed().as_secs();
            let time_str = if elapsed < 60 {
                format!("{:>2}s", elapsed)
            } else if elapsed < 3600 {
                format!("{:>2}m", elapsed / 60)
            } else {
                format!("{:>2}h", elapsed / 3600)
            };

            let content_color = if s.contains("error") || s.contains("Error") || s.contains("ERROR")
            {
                THEME_ERROR
            } else if s.contains("warn") || s.contains("Warn") || s.contains("WARN") {
                THEME_WARNING
            } else if s.contains("info") || s.contains("Info") || s.contains("INFO") {
                THEME_PRIMARY
            } else if i < 5 {
                THEME_TEXT
            } else {
                THEME_MUTED
            };

            let line = Line::from(vec![
                Span::styled(time_str, Style::default().fg(THEME_MUTED)),
                Span::styled(" │ ", Style::default().fg(THEME_MUTED)),
                Span::styled(s, Style::default().fg(content_color)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let status_indicator = if paused {
        "⏸️ PAUSED"
    } else {
        "▶️ LIVE"
    };
    let filter_indicator = if let Some(f) = filter {
        format!(" 🔍 '{}'", f)
    } else {
        String::new()
    };

    let title = format!(
        "📝 Events {} ( {}/{} ){}",
        status_indicator,
        filtered_events.len(),
        events.len(),
        filter_indicator
    );

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if paused { THEME_WARNING } else { THEME_PRIMARY }))
            .title(title)
            .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(list, area);
}

pub fn draw_component_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    components: &[String],
    state: &mut ListState,
    lines: &VecDeque<String>,
) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .margin(1)
        .constraints([Constraint::Length(30), Constraint::Min(1)])
        .split(area);

    let items: Vec<ListItem> = components
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let icon = if i == 0 { "🏠" } else { "🧩" };
            ListItem::new(Line::from(vec![
                Span::styled(icon, Style::default().fg(THEME_PRIMARY)),
                Span::styled(format!(" {}", c), Style::default().fg(THEME_TEXT)),
            ]))
        })
        .collect();

    let components_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(THEME_PRIMARY))
                .title("🧩 Components")
                .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
        )
        .highlight_style(
            Style::default()
                .bg(THEME_PRIMARY)
                .fg(THEME_BACKGROUND)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(components_list, layout[0], state);

    let log_items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let content_color = if line.contains("[ERROR]") || line.contains("ERROR") {
                THEME_ERROR
            } else if line.contains("[WARN]") || line.contains("WARN") {
                THEME_WARNING
            } else if line.contains("[INFO]") || line.contains("INFO") {
                THEME_PRIMARY
            } else if line.contains("[DEBUG]") || line.contains("DEBUG") {
                THEME_MUTED
            } else if i < 10 {
                THEME_TEXT
            } else {
                THEME_MUTED
            };

            ListItem::new(Line::from(Span::styled(
                line,
                Style::default().fg(content_color),
            )))
        })
        .collect();

    let selected_component = components
        .get(state.selected().unwrap_or(0))
        .map(|s| s.as_str())
        .unwrap_or("None");

    let logs_list = List::new(log_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(THEME_ACCENT))
            .title(format!("📋 Logs - {}", selected_component))
            .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(logs_list, layout[1]);
}
