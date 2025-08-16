use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use libp2p::PeerId;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Sparkline, Table, TableState,
    },
};

use super::{PeerRow, View, EVENTS_CAP};

pub fn draw_top(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    view: &View,
    peer_count: usize,
    link_count: usize,
    local_peer_id: &PeerId,
) {
    let time = chrono::Local::now().format("%H:%M:%S");
    let title = format!(
        " realm | {} | peers:{} | links:{} | {} | {} ",
        match view {
            View::Overview => "overview",
            View::Peers => "peers",
            View::Deployments => "deployments",
            View::Topology => "topology",
            View::Events => "events",
            View::Logs => "logs",
            View::Ops => "ops",
        },
        peer_count,
        link_count,
        local_peer_id,
        time
    );
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .title(Span::styled(title, Style::default().fg(Color::Cyan)));
    let p = Paragraph::new("").block(block);
    f.render_widget(p, area);
}

pub fn draw_nav(f: &mut ratatui::Frame<'_>, area: Rect, items: &[&str], selected: usize) {
    let list = List::new(
        items
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                ListItem::new(Span::styled(*s, style))
            })
            .collect::<Vec<_>>(),
    )
    .block(Block::default().borders(Borders::RIGHT));
    f.render_widget(list, area);
}

pub fn draw_footer(f: &mut ratatui::Frame<'_>, area: Rect) {
    let help =
        "q quit  ↑/↓ or j/k tabs  1..7 jump  c collapse  A apply  U upgrade  W run  / filter (+addr to dial)  p pause";
    let p = Paragraph::new(Line::from(help))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, area);
}

pub fn draw_overview(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    cpu: &[u64],
    mem: &[u64],
    msgs: &[u64],
    peer_count: usize,
    events: &VecDeque<(Instant, String)>,
    components_desired_total: u64,
    components_running_total: u64,
    restarts_total: u64,
    publish_errors_total: u64,
    fuel_used_total: u64,
    mem_current_bytes: u64,
    mem_peak_bytes: u64,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(1),
        ])
        .split(area);

    let tiles = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(18),
            Constraint::Length(22),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(26),
            Constraint::Length(34),
            Constraint::Min(1),
        ])
        .split(rows[0]);

    let tile_style = Style::default().fg(Color::Gray);
    let t1 = Paragraph::new(format!("Peers: {peer_count}"))
        .style(tile_style)
        .block(Block::default().borders(Borders::ALL).title("Health"));
    f.render_widget(t1, tiles[0]);

    let s_cpu = Sparkline::default()
        .data(cpu)
        .style(Style::default().fg(Color::LightGreen))
        .max(100)
        .block(Block::default().borders(Borders::ALL).title("CPU%"));
    let s_mem = Sparkline::default()
        .data(mem)
        .style(Style::default().fg(Color::LightMagenta))
        .max(100)
        .block(Block::default().borders(Borders::ALL).title("MEM%"));
    let s_msg = Sparkline::default()
        .data(msgs)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("msgs/s"));
    let spark_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Min(10),
            Constraint::Min(10),
        ])
        .split(rows[1]);
    f.render_widget(s_cpu, spark_row[0]);
    f.render_widget(s_mem, spark_row[1]);
    f.render_widget(s_msg, spark_row[2]);

    // Components tile: desired/running/drift
    let drift = components_desired_total.saturating_sub(components_running_total);
    let comps = Paragraph::new(format!(
        "Desired: {}  Running: {}  Drift: {}",
        components_desired_total, components_running_total, drift
    ))
    .style(tile_style)
    .block(Block::default().borders(Borders::ALL).title("Components"));
    f.render_widget(comps, tiles[4]);

    // Stats tile: restarts, publish errors, fuel, mem
    let mem_cur_mb = mem_current_bytes / (1024 * 1024);
    let mem_peak_mb = mem_peak_bytes / (1024 * 1024);
    let stats = Paragraph::new(format!(
        "Restarts: {}  PubErr: {}\nFuel: {}  Mem: {} / {} MB",
        restarts_total, publish_errors_total, fuel_used_total, mem_cur_mb, mem_peak_mb
    ))
    .style(tile_style)
    .block(Block::default().borders(Borders::ALL).title("Stats"));
    f.render_widget(stats, tiles[5]);

    let list_items: Vec<ListItem> = events
        .iter()
        .take(50)
        .map(|(_, s)| ListItem::new(s.clone()))
        .collect();
    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Recent events"),
    );
    f.render_widget(list, rows[2]);
}

pub fn draw_peers(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    peers: &BTreeMap<String, PeerRow>,
    peer_latency: &BTreeMap<String, u128>,
    state: &mut TableState,
) {
    let cols = ["Peer ID", "Agent", "RTT(ms)", "Last ping", "Tags", "Drift"];
    let header = ratatui::widgets::Row::new(cols.iter().map(|h| {
        Line::from(*h).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    }));
    let mut rows = Vec::new();
    for (id, p) in peers.iter() {
        let secs = p.last_ping.elapsed().as_secs();
        let rtt = peer_latency.get(id).cloned().unwrap_or_default();
        let drift = p
            .desired_components
            .saturating_sub(p.running_components);
        let mut row = ratatui::widgets::Row::new(vec![
            id.clone(),
            p.agent_version.to_string(),
            rtt.to_string(),
            format!("{}s", secs),
            p.roles.clone(),
            drift.to_string(),
        ]);
        if drift > 0 {
            row = row.style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        }
        rows.push(row);
    }
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Percentage(24),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Peers"))
    .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    f.render_stateful_widget(table, area, state);
}

pub fn draw_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    events: &VecDeque<(Instant, String)>,
    filter: Option<&str>,
    paused: bool,
) {
    let items: Vec<ListItem> = events
        .iter()
        .filter(|(_, s)| filter.map_or(true, |f| s.contains(f)))
        .map(|(t, s)| ListItem::new(format!("{:>4}s | {}", t.elapsed().as_secs(), s)))
        .collect();
    let title = if paused {
        format!("Events [PAUSED] ( {}/{} )", events.len(), EVENTS_CAP)
    } else {
        format!("Events ( {}/{} )", events.len(), EVENTS_CAP)
    };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);
}

pub fn draw_component_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    components: &[String],
    state: &mut ListState,
    lines: &VecDeque<String>,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(1)])
        .split(area);
    let items: Vec<ListItem> = components
        .iter()
        .map(|c| ListItem::new(c.clone()))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Components"))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    f.render_stateful_widget(list, cols[0], state);
    let log_items: Vec<ListItem> = lines.iter().map(|l| ListItem::new(l.clone())).collect();
    let logs = List::new(log_items).block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(logs, cols[1]);
}

pub fn draw_placeholder(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
    let p = Paragraph::new(text).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

pub fn draw_topology(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    topo: &BTreeMap<String, (Option<String>, Instant)>,
) {
    let cols = ["Peer ID", "Last seen", "Addr"];
    let header = ratatui::widgets::Row::new(cols.iter().map(|h| {
        Line::from(*h).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    }));
    let mut rows = Vec::new();
    for (peer, (addr, last)) in topo.iter() {
        let secs = last.elapsed().as_secs();
        rows.push(ratatui::widgets::Row::new(vec![
            peer.clone(),
            format!("{}s", secs),
            addr.clone().unwrap_or_default(),
        ]));
    }
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(45),
            Constraint::Length(10),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Topology (mDNS)"),
    )
    .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    f.render_widget(table, area);
}

pub fn draw_overlay(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
    let popup = Rect {
        x: area.x + area.width / 4,
        y: area.y + area.height / 3,
        width: area.width / 2,
        height: 3,
    };
    let block = Block::default().borders(Borders::ALL).title("");
    let p = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(Clear, popup);
    f.render_widget(p, popup);
}
