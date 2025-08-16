use std::collections::BTreeMap;
use std::time::Instant;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph, Table},
};

use super::{
    THEME_ACCENT, THEME_BACKGROUND, THEME_MUTED, THEME_PRIMARY, THEME_SUCCESS, THEME_TEXT,
    THEME_WARNING,
};

pub fn draw_topology(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    topo: &BTreeMap<String, (Option<String>, Instant)>,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(4), Constraint::Min(1)])
        .split(area);

    let summary_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(layout[0]);

    let total_discovered = topo.len();
    let active_peers = topo
        .values()
        .filter(|(_, last)| last.elapsed().as_secs() < 60)
        .count();
    let has_addresses = topo.values().filter(|(addr, _)| addr.is_some()).count();
    let recent_discoveries = topo
        .values()
        .filter(|(_, last)| last.elapsed().as_secs() < 10)
        .count();

    let discovered_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME_PRIMARY))
        .title("🔍 Discovered")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let discovered_para = Paragraph::new(format!("{}", total_discovered))
        .style(Style::default().fg(THEME_TEXT))
        .block(discovered_block)
        .alignment(Alignment::Center);
    f.render_widget(discovered_para, summary_layout[0]);

    let active_color = if active_peers > 0 {
        THEME_SUCCESS
    } else {
        THEME_WARNING
    };
    let active_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(active_color))
        .title("🟢 Active")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let active_para = Paragraph::new(format!("{}", active_peers))
        .style(Style::default().fg(THEME_TEXT))
        .block(active_block)
        .alignment(Alignment::Center);
    f.render_widget(active_para, summary_layout[1]);

    let addr_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME_ACCENT))
        .title("📍 Addressable")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let addr_para = Paragraph::new(format!("{}", has_addresses))
        .style(Style::default().fg(THEME_TEXT))
        .block(addr_block)
        .alignment(Alignment::Center);
    f.render_widget(addr_para, summary_layout[2]);

    let recent_color = if recent_discoveries > 0 {
        THEME_SUCCESS
    } else {
        THEME_MUTED
    };
    let recent_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(recent_color))
        .title("🆕 Recent")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let recent_para = Paragraph::new(format!("{}", recent_discoveries))
        .style(Style::default().fg(THEME_TEXT))
        .block(recent_block)
        .alignment(Alignment::Center);
    f.render_widget(recent_para, summary_layout[3]);

    let cols = ["🏷️ Peer ID", "⏰ Last Seen", "📡 Address"];
    let header = ratatui::widgets::Row::new(cols.iter().map(|h| {
        Line::from(*h).style(
            Style::default()
                .fg(THEME_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    }));

    let mut rows = Vec::new();
    for (peer, (addr, last)) in topo.iter() {
        let secs = last.elapsed().as_secs();
        let time_str = if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m", secs / 60)
        } else {
            format!("{}h", secs / 3600)
        };

        let status_indicator = if secs < 10 {
            "🟢"
        } else if secs < 60 {
            "🟡"
        } else {
            "🔴"
        };

        let row_style = if secs < 60 {
            Style::default().fg(THEME_TEXT)
        } else {
            Style::default().fg(THEME_MUTED)
        };

        rows.push(
            ratatui::widgets::Row::new(vec![
                format!("{:.12}...", peer),
                format!("{} {}", status_indicator, time_str),
                addr.clone().unwrap_or_else(|| "N/A".to_string()),
            ])
            .style(row_style),
        );
    }

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(15),
            Constraint::Percentage(55),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(THEME_PRIMARY))
            .title("🌐 Network Topology (mDNS)")
            .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
    )
    .highlight_style(
        Style::default()
            .bg(THEME_PRIMARY)
            .fg(THEME_BACKGROUND)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(table, layout[1]);
}
