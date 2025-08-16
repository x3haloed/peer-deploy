use std::collections::BTreeMap;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph, Table, TableState},
};

use crate::tui::state::PeerRow;

use super::ThemeColors;

pub fn draw_deployments(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    peers: &BTreeMap<String, PeerRow>,
    state: &mut TableState,
    theme: &ThemeColors,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(4), Constraint::Min(1)])
        .split(area);

    // Summary cards
    let totals_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(layout[0]);

    let desired_total: u64 = peers.values().map(|p| p.desired_components).sum();
    let running_total: u64 = peers.values().map(|p| p.running_components).sum();
    let drift_total = desired_total.saturating_sub(running_total);

    let desired_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary))
        .title("📦 Desired")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let desired_para = Paragraph::new(format!("{}", desired_total))
        .style(Style::default().fg(theme.text))
        .block(desired_block)
        .alignment(Alignment::Center);
    f.render_widget(desired_para, totals_layout[0]);

    let running_color = if running_total >= desired_total {
        theme.success
    } else {
        theme.warning
    };
    let running_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(running_color))
        .title("🚀 Running")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let running_para = Paragraph::new(format!("{}", running_total))
        .style(Style::default().fg(theme.text))
        .block(running_block)
        .alignment(Alignment::Center);
    f.render_widget(running_para, totals_layout[1]);

    let drift_color = if drift_total == 0 { theme.success } else { theme.error };
    let drift_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(drift_color))
        .title("⚠️ Drift")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let drift_para = Paragraph::new(format!("{}", drift_total))
        .style(Style::default().fg(theme.text))
        .block(drift_block)
        .alignment(Alignment::Center);
    f.render_widget(drift_para, totals_layout[2]);

    // Table of peers deployments
    let cols = [
        "🏷️ Peer ID",
        "🎯 Desired",
        "🚀 Running",
        "⚠️ Drift",
        "🏷️ Tags",
        "💓 Last Ping",
    ];
    let header = ratatui::widgets::Row::new(cols.iter().map(|h| {
        Line::from(*h).style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )
    }));

    let mut rows = Vec::new();
    for (id, p) in peers.iter() {
        let secs = p.last_ping.elapsed().as_secs();
        let drift = p.desired_components.saturating_sub(p.running_components);

        let health_style = if secs > 30 {
            Style::default().fg(theme.error)
        } else if drift > 0 {
            Style::default().fg(theme.warning)
        } else {
            Style::default().fg(theme.text)
        };

        let ping_indicator = if secs < 10 {
            "🟢"
        } else if secs < 30 {
            "🟡"
        } else {
            "🔴"
        };

        let row = ratatui::widgets::Row::new(vec![
            format!("{:.12}...", id),
            format!("{}", p.desired_components),
            format!("{}", p.running_components),
            format!("{}", drift),
            p.roles.clone(),
            format!("{} {}s", ping_indicator, secs),
        ])
        .style(health_style);
        rows.push(row);
    }

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Percentage(30),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title("🚀 Application Deployments")
            .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
    )
    .highlight_style(
        Style::default()
            .bg(theme.primary)
            .fg(theme.background)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(table, layout[1], state);
}


