use std::collections::VecDeque;
use std::time::Instant;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use super::ThemeColors;

pub fn draw_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    events: &VecDeque<(Instant, String)>,
    filter: Option<&str>,
    paused: bool,
    theme: &ThemeColors,
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
                theme.error
            } else if s.contains("warn") || s.contains("Warn") || s.contains("WARN") {
                theme.warning
            } else if s.contains("info") || s.contains("Info") || s.contains("INFO") {
                theme.primary
            } else if i < 5 {
                theme.text
            } else {
                theme.muted
            };

            let line = Line::from(vec![
                Span::styled(time_str, Style::default().fg(theme.muted)),
                Span::styled(" • ", Style::default().fg(theme.muted)),
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
            .borders(Borders::NONE)
            .title(title)
            .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(list, area);
}

pub fn draw_component_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    components: &[String],
    state: &mut ListState,
    lines: &VecDeque<String>,
    theme: &ThemeColors,
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
                Span::styled(icon, Style::default().fg(theme.primary)),
                Span::styled(format!(" {}", c), Style::default().fg(theme.text)),
            ]))
        })
        .collect();

    let components_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .title("🧩 Components")
                .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        )
        .highlight_style(
            Style::default()
                .bg(theme.primary)
                .fg(theme.background)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(components_list, layout[0], state);

    let log_items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let content_color = if line.contains("[ERROR]") || line.contains("ERROR") {
                theme.error
            } else if line.contains("[WARN]") || line.contains("WARN") {
                theme.warning
            } else if line.contains("[INFO]") || line.contains("INFO") {
                theme.primary
            } else if line.contains("[DEBUG]") || line.contains("DEBUG") {
                theme.muted
            } else if i < 10 {
                theme.text
            } else {
                theme.muted
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
            .borders(Borders::NONE)
            .title(format!("📋 Logs - {}", selected_component))
            .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(logs_list, layout[1]);
}
