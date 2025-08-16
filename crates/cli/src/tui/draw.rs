use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use libp2p::PeerId;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline,
        Table, TableState, Tabs, Wrap,
    },
};

use crate::tui::state::{PeerRow, View};

// Color scheme constants for consistent theming
pub const THEME_PRIMARY: Color = Color::Rgb(79, 172, 254); // Blue
pub const THEME_SUCCESS: Color = Color::Rgb(34, 197, 94); // Green
pub const THEME_WARNING: Color = Color::Rgb(251, 191, 36); // Yellow
pub const THEME_ERROR: Color = Color::Rgb(239, 68, 68); // Red
pub const THEME_MUTED: Color = Color::Rgb(156, 163, 175); // Gray
pub const THEME_ACCENT: Color = Color::Rgb(168, 85, 247); // Purple
pub const THEME_BACKGROUND: Color = Color::Rgb(17, 24, 39); // Dark blue-gray
pub const THEME_SURFACE: Color = Color::Rgb(31, 41, 55); // Lighter blue-gray
pub const THEME_TEXT: Color = Color::Rgb(243, 244, 246); // Light gray

pub fn draw_header_tabs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    view: &View,
    peer_count: usize,
    link_count: usize,
    local_peer_id: &PeerId,
) {
    let time = chrono::Local::now().format("%H:%M:%S");

    // Split header into title row and tabs row
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Title bar with status information
    let status_info = format!(
        " realm-tui │ {} peers │ {} links │ {} │ {} ",
        peer_count,
        link_count,
        format!("{:.8}...", local_peer_id.to_string()),
        time
    );

    let title_block = Block::default()
        .style(Style::default().bg(THEME_SURFACE))
        .borders(Borders::NONE);

    let title_para = Paragraph::new(status_info)
        .style(Style::default().fg(THEME_MUTED))
        .block(title_block);
    f.render_widget(title_para, chunks[0]);

    // Tab navigation
    let tab_titles = vec![
        "📊 Overview",
        "👥 Peers",
        "🚀 Apps",
        "🌐 Network",
        "📝 Events",
        "📋 Logs",
        "⚙️  Actions",
    ];

    let selected_tab = match view {
        View::Overview => 0,
        View::Peers => 1,
        View::Deployments => 2,
        View::Topology => 3,
        View::Events => 4,
        View::Logs => 5,
        View::Ops => 6,
    };

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(THEME_PRIMARY)),
        )
        .style(Style::default().fg(THEME_MUTED))
        .highlight_style(
            Style::default()
                .fg(THEME_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
        .select(selected_tab);

    f.render_widget(tabs, chunks[1]);
}

// Navigation sidebar is replaced by header tabs - this function is deprecated

pub fn draw_footer(f: &mut ratatui::Frame<'_>, area: Rect) {
    let shortcuts = vec![
        ("Tab/1-7", "Navigate"),
        ("A", "Apply"),
        ("D", "Deploy"),
        ("U", "Upgrade"),
        ("I", "Install"),
        ("/", "Filter"),
        ("ESC", "Cancel"),
        ("Q", "Quit"),
    ];

    let help_text: Vec<Span> = shortcuts
        .iter()
        .enumerate()
        .flat_map(|(i, (key, desc))| {
            let mut spans = vec![
                Span::styled(
                    key.to_string(),
                    Style::default()
                        .fg(THEME_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}", desc), Style::default().fg(THEME_MUTED)),
            ];
            if i < shortcuts.len() - 1 {
                spans.push(Span::styled(
                    "  │  ".to_string(),
                    Style::default().fg(THEME_MUTED),
                ));
            }
            spans
        })
        .collect();

    let footer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(THEME_MUTED))
        .style(Style::default().bg(THEME_SURFACE));

    let footer_para = Paragraph::new(Line::from(help_text))
        .alignment(Alignment::Center)
        .block(footer_block)
        .wrap(Wrap { trim: true });

    f.render_widget(footer_para, area);
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
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(6), // Status cards
            Constraint::Length(8), // Charts
            Constraint::Min(10),   // Events/Logs
        ])
        .split(area);

    // Status Cards Row
    let cards_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20), // Cluster Health
            Constraint::Percentage(25), // Components
            Constraint::Percentage(25), // System Stats
            Constraint::Percentage(30), // Resource Usage
        ])
        .split(main_layout[0]);

    // Cluster Health Card
    let health_color = if peer_count > 0 {
        THEME_SUCCESS
    } else {
        THEME_ERROR
    };
    let health_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(health_color))
        .title("🏥 Cluster Health")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));

    let health_content = format!(
        "Peers: {}\nStatus: {}",
        peer_count,
        if peer_count > 0 {
            "Healthy"
        } else {
            "No Peers"
        }
    );

    let health_para = Paragraph::new(health_content)
        .style(Style::default().fg(THEME_TEXT))
        .block(health_block)
        .alignment(Alignment::Center);
    f.render_widget(health_para, cards_layout[0]);

    // Components Card
    let drift = components_desired_total.saturating_sub(components_running_total);
    let comp_color = if drift == 0 {
        THEME_SUCCESS
    } else {
        THEME_WARNING
    };
    let comp_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(comp_color))
        .title("🚀 Applications")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));

    let comp_content = format!(
        "Desired: {}\nRunning: {}\nDrift: {}",
        components_desired_total, components_running_total, drift
    );

    let comp_para = Paragraph::new(comp_content)
        .style(Style::default().fg(THEME_TEXT))
        .block(comp_block)
        .alignment(Alignment::Center);
    f.render_widget(comp_para, cards_layout[1]);

    // System Stats Card
    let stats_color = if publish_errors_total > 10 {
        THEME_ERROR
    } else {
        THEME_PRIMARY
    };
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(stats_color))
        .title("📊 System Stats")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));

    let stats_content = format!(
        "Restarts: {}\nErrors: {}\nFuel: {}",
        restarts_total, publish_errors_total, fuel_used_total
    );

    let stats_para = Paragraph::new(stats_content)
        .style(Style::default().fg(THEME_TEXT))
        .block(stats_block)
        .alignment(Alignment::Center);
    f.render_widget(stats_para, cards_layout[2]);

    // Resource Usage Card with Gauges
    let mem_cur_mb = mem_current_bytes / (1024 * 1024);
    let mem_peak_mb = mem_peak_bytes / (1024 * 1024);
    let mem_usage_ratio = if mem_peak_mb > 0 {
        (mem_cur_mb as f64 / mem_peak_mb as f64 * 100.0) as u16
    } else {
        0
    };

    let resource_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME_ACCENT))
        .title("💾 Resources")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));

    let resource_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(resource_block.inner(cards_layout[3]));

    f.render_widget(resource_block, cards_layout[3]);

    // Memory gauge
    let mem_gauge = Gauge::default()
        .block(Block::default().title(format!("Memory: {} MB", mem_cur_mb)))
        .gauge_style(Style::default().fg(THEME_ACCENT))
        .ratio(mem_usage_ratio.min(100) as f64 / 100.0);
    f.render_widget(mem_gauge, resource_layout[0]);

    // Performance Charts Row
    let charts_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_layout[1]);

    // CPU Sparkline
    let cpu_color = if cpu.last().unwrap_or(&0) > &80 {
        THEME_ERROR
    } else {
        THEME_SUCCESS
    };
    let cpu_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(cpu_color))
                .title("📈 CPU Usage")
                .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
        )
        .data(cpu)
        .style(Style::default().fg(cpu_color))
        .max(100);
    f.render_widget(cpu_sparkline, charts_layout[0]);

    // Memory Sparkline
    let mem_color = if mem.last().unwrap_or(&0) > &80 {
        THEME_ERROR
    } else {
        THEME_PRIMARY
    };
    let mem_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(mem_color))
                .title("🧠 Memory Usage")
                .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
        )
        .data(mem)
        .style(Style::default().fg(mem_color))
        .max(100);
    f.render_widget(mem_sparkline, charts_layout[1]);

    // Messages Sparkline
    let msg_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(THEME_WARNING))
                .title("📡 Messages/sec")
                .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
        )
        .data(msgs)
        .style(Style::default().fg(THEME_WARNING));
    f.render_widget(msg_sparkline, charts_layout[2]);

    // Recent Events
    let events_items: Vec<ListItem> = events
        .iter()
        .take(50)
        .enumerate()
        .map(|(i, (time, msg))| {
            let time_ago = format!("{:>3}s", time.elapsed().as_secs());
            let styled_line = Line::from(vec![
                Span::styled(time_ago, Style::default().fg(THEME_MUTED)),
                Span::styled(" │ ", Style::default().fg(THEME_MUTED)),
                Span::styled(
                    msg,
                    Style::default().fg(if i < 5 { THEME_TEXT } else { THEME_MUTED }),
                ),
            ]);
            ListItem::new(styled_line)
        })
        .collect();

    let events_list = List::new(events_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(THEME_PRIMARY))
            .title("📝 Recent Events")
            .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(events_list, main_layout[2]);
}

pub fn draw_peers(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    peers: &BTreeMap<String, PeerRow>,
    peer_latency: &BTreeMap<String, u128>,
    state: &mut TableState,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(4), Constraint::Min(1)])
        .split(area);

    // Summary stats at top
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

    // Total Peers Card
    let peers_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME_PRIMARY))
        .title("👥 Total Peers")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let peers_para = Paragraph::new(format!("{}", total_peers))
        .style(Style::default().fg(THEME_TEXT))
        .block(peers_block)
        .alignment(Alignment::Center);
    f.render_widget(peers_para, summary_layout[0]);

    // Healthy Peers Card
    let health_color = if healthy_peers == total_peers && total_peers > 0 {
        THEME_SUCCESS
    } else {
        THEME_WARNING
    };
    let health_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(health_color))
        .title("💚 Healthy")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let health_para = Paragraph::new(format!("{}/{}", healthy_peers, total_peers))
        .style(Style::default().fg(THEME_TEXT))
        .block(health_block)
        .alignment(Alignment::Center);
    f.render_widget(health_para, summary_layout[1]);

    // Drift Card
    let drift_color = if total_drift == 0 {
        THEME_SUCCESS
    } else {
        THEME_ERROR
    };
    let drift_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(drift_color))
        .title("⚠️ Drift")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let drift_para = Paragraph::new(format!("{}", total_drift))
        .style(Style::default().fg(THEME_TEXT))
        .block(drift_block)
        .alignment(Alignment::Center);
    f.render_widget(drift_para, summary_layout[2]);

    // Avg RTT Card
    let rtt_color = if avg_rtt < 100 {
        THEME_SUCCESS
    } else if avg_rtt < 500 {
        THEME_WARNING
    } else {
        THEME_ERROR
    };
    let rtt_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rtt_color))
        .title("📡 Avg RTT")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));
    let rtt_para = Paragraph::new(format!("{}ms", avg_rtt))
        .style(Style::default().fg(THEME_TEXT))
        .block(rtt_block)
        .alignment(Alignment::Center);
    f.render_widget(rtt_para, summary_layout[3]);

    // Peers table
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
                .fg(THEME_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    }));

    let mut rows = Vec::new();
    for (id, p) in peers.iter() {
        let secs = p.last_ping.elapsed().as_secs();
        let rtt = peer_latency.get(id).cloned().unwrap_or_default();
        let drift = p.desired_components.saturating_sub(p.running_components);

        // Determine row style based on health
        let row_style = if secs > 30 {
            Style::default().fg(THEME_ERROR)
        } else if drift > 0 {
            Style::default().fg(THEME_WARNING)
        } else {
            Style::default().fg(THEME_TEXT)
        };

        // Status indicators
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
            .border_style(Style::default().fg(THEME_PRIMARY))
            .title("👥 Peer Details")
            .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD)),
    )
    .highlight_style(
        Style::default()
            .bg(THEME_PRIMARY)
            .fg(THEME_BACKGROUND)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(table, layout[1], state);
}

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

            // Color code based on content
            let content_color = if s.contains("error") || s.contains("Error") || s.contains("ERROR")
            {
                THEME_ERROR
            } else if s.contains("warn") || s.contains("Warn") || s.contains("WARN") {
                THEME_WARNING
            } else if s.contains("info") || s.contains("Info") || s.contains("INFO") {
                THEME_PRIMARY
            } else if i < 5 {
                THEME_TEXT // Recent events more prominent
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

    // Components list
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

    // Component logs
    let log_items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            // Try to parse log level from line
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

pub fn draw_placeholder(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
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
        .border_style(Style::default().fg(THEME_MUTED))
        .title("🚧 Coming Soon")
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD));

    let content = Paragraph::new(text)
        .style(Style::default().fg(THEME_MUTED))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(block);

    f.render_widget(content, center_layout[1]);
}

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

    // Network stats summary
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

    // Total Discovered
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

    // Active Peers
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

    // With Addresses
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

    // Recent
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

    // Topology table
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

pub fn draw_overlay(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
    // Calculate popup size based on content
    let text_width = text.len().min(60) + 4;
    let text_lines = (text.len() / 50) + 1;
    let popup_height = (text_lines + 2).min(8);

    let popup = Rect {
        x: area.x + (area.width.saturating_sub(text_width as u16)) / 2,
        y: area.y + area.height / 3,
        width: text_width as u16,
        height: popup_height as u16,
    };

    // Semi-transparent background effect
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

pub fn draw_wizard_dialog(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &str,
    step: usize,
    total_steps: usize,
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

    // Progress indicator
    let progress_text = format!("Step {} of {}", step + 1, total_steps);
    let progress = (step as f64 / total_steps as f64) * popup_width as f64;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME_PRIMARY))
        .title(format!("🧙 {} - {}", title, progress_text))
        .title_style(Style::default().fg(THEME_TEXT).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(THEME_SURFACE));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1), // Progress bar
            Constraint::Length(2), // Prompt
            Constraint::Length(3), // Input
            Constraint::Min(1),    // Help text
        ])
        .split(inner);

    // Progress bar
    let progress_gauge = Gauge::default()
        .gauge_style(Style::default().fg(THEME_PRIMARY))
        .ratio(progress / popup_width as f64);
    f.render_widget(progress_gauge, layout[0]);

    // Prompt
    let prompt_para = Paragraph::new(prompt)
        .style(Style::default().fg(THEME_TEXT))
        .wrap(Wrap { trim: true });
    f.render_widget(prompt_para, layout[1]);

    // Input field
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME_ACCENT))
        .title("Input");

    let input_para = Paragraph::new(format!("{}_", input))
        .style(Style::default().fg(THEME_TEXT))
        .block(input_block);
    f.render_widget(input_para, layout[2]);

    // Help text
    let help_text = "Enter: Confirm  |  Esc: Cancel  |  Backspace: Delete";
    let help_para = Paragraph::new(help_text)
        .style(Style::default().fg(THEME_MUTED))
        .alignment(Alignment::Center);
    f.render_widget(help_para, layout[3]);
}
