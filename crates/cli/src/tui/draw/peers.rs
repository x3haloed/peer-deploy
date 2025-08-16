use std::collections::BTreeMap;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph, Table, TableState},
};

use crate::tui::state::PeerRow;

use super::ThemeColors;

pub fn draw_peers(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    peers: &BTreeMap<String, PeerRow>,
    peer_latency: &BTreeMap<String, u128>,
    state: &mut TableState,
    theme: &ThemeColors,
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

    let total_peers = peers.len();
    let healthy_peers = peers
        .values()
        .filter(|p| p.last_ping.elapsed().as_secs() < 30)
        .count();
    let total_drift: u64 = peers
        .values()
        .map(|p| p.desired_components.saturating_sub(p.running_components))
        .sum();
    let avg_rtt = if !peer_latency.is_empty() {
        peer_latency.values().sum::<u128>() / peer_latency.len() as u128
    } else {
        0
    };

    let peers_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary))
        .title("👥 Total Peers")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let peers_para = Paragraph::new(format!("{}", total_peers))
        .style(Style::default().fg(theme.text))
        .block(peers_block)
        .alignment(Alignment::Center);
    f.render_widget(peers_para, summary_layout[0]);

    let health_color = if healthy_peers == total_peers && total_peers > 0 {
        theme.success
    } else {
        theme.warning
    };
    let health_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(health_color))
        .title("💚 Healthy")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let health_para = Paragraph::new(format!("{}/{}", healthy_peers, total_peers))
        .style(Style::default().fg(theme.text))
        .block(health_block)
        .alignment(Alignment::Center);
    f.render_widget(health_para, summary_layout[1]);

    let drift_color = if total_drift == 0 {
        theme.success
    } else {
        theme.error
    };
    let drift_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(drift_color))
        .title("⚠️ Drift")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let drift_para = Paragraph::new(format!("{}", total_drift))
        .style(Style::default().fg(theme.text))
        .block(drift_block)
        .alignment(Alignment::Center);
    f.render_widget(drift_para, summary_layout[2]);

    let rtt_color = if avg_rtt < 100 {
        theme.success
    } else if avg_rtt < 500 {
        theme.warning
    } else {
        theme.error
    };
    let rtt_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rtt_color))
        .title("📡 Avg RTT")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    let rtt_para = Paragraph::new(format!("{}ms", avg_rtt))
        .style(Style::default().fg(theme.text))
        .block(rtt_block)
        .alignment(Alignment::Center);
    f.render_widget(rtt_para, summary_layout[3]);

    let cols = [
        "🏷️ Peer ID",
        "🔧 Version",
        "📡 RTT",
        "💓 Last Ping",
        "🏷️ Tags",
        "⚠️ Drift",
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
        let rtt = peer_latency.get(id).cloned().unwrap_or_default();
        let drift = p.desired_components.saturating_sub(p.running_components);

        let row_style = if secs > 30 {
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
        let drift_indicator = if drift == 0 { "✅" } else { "⚠️" };

        let row = ratatui::widgets::Row::new(vec![
            format!("{:.12}...", id),
            format!("v{}", p.agent_version),
            format!("{}ms", rtt),
            format!("{} {}s", ping_indicator, secs),
            p.roles.clone(),
            format!("{} {}", drift_indicator, drift),
        ])
        .style(row_style);

        rows.push(row);
    }

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Percentage(30),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.primary))
            .title("👥 Peer Details")
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
