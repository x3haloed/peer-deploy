use std::collections::VecDeque;
use std::time::Instant;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Sparkline},
};

use super::ThemeColors;

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
    theme: &ThemeColors,
) {
    // Avoid global background to prevent red fallback; rely on borders and text colors only.
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(10),
        ])
        .split(area);

    let cards_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
        ])
        .split(main_layout[0]);

    let health_color = if peer_count > 0 {
        theme.success
    } else {
        theme.error
    };
    let health_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(health_color))
        .title("🏥 Cluster Health")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));

    let health_content = format!("Peers: {}\nLinks: {}\nStatus: {}",
        peer_count,
        msgs.last().cloned().unwrap_or(0),
        if peer_count > 0 { "Healthy" } else { "No Peers" }
    );

    let health_para = Paragraph::new(health_content)
        .style(Style::default().fg(theme.text))
        .block(health_block)
        .alignment(Alignment::Center);
    f.render_widget(health_para, cards_layout[0]);

    let drift = components_desired_total.saturating_sub(components_running_total);
    let comp_color = if drift == 0 {
        theme.success
    } else {
        theme.warning
    };
    let comp_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(comp_color))
        .title("🚀 Applications")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));

    let comp_content = format!(
        "Desired: {}\nRunning: {}\nDrift: {}",
        components_desired_total, components_running_total, drift
    );

    let comp_para = Paragraph::new(comp_content)
        .style(Style::default().fg(theme.text))
        .block(comp_block)
        .alignment(Alignment::Center);
    f.render_widget(comp_para, cards_layout[1]);

    let stats_color = if publish_errors_total > 10 {
        theme.error
    } else {
        theme.primary
    };
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(stats_color))
        .title("📊 System Stats")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));

    let stats_content = format!(
        "Restarts: {}\nErrors: {}\nFuel: {}",
        restarts_total, publish_errors_total, fuel_used_total
    );

    let stats_para = Paragraph::new(stats_content)
        .style(Style::default().fg(theme.text))
        .block(stats_block)
        .alignment(Alignment::Center);
    f.render_widget(stats_para, cards_layout[2]);

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
        .border_style(Style::default().fg(theme.accent))
        .title("💾 Resources")
        .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));

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

    let mem_gauge = Gauge::default()
        .block(Block::default().title(format!("Memory: {} MB", mem_cur_mb)))
        .gauge_style(Style::default().fg(theme.accent))
        .ratio(mem_usage_ratio.min(100) as f64 / 100.0);
    f.render_widget(mem_gauge, resource_layout[0]);

    let charts_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_layout[1]);

    let cpu_color = if cpu.last().unwrap_or(&0) > &80 {
        theme.error
    } else {
        theme.success
    };
    let cpu_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(cpu_color))
                .title("📈 CPU Usage")
                .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        )
        .data(cpu)
        .style(Style::default().fg(cpu_color))
        .max(100);
    f.render_widget(cpu_sparkline, charts_layout[0]);

    let mem_color = if mem.last().unwrap_or(&0) > &80 {
        theme.error
    } else {
        theme.primary
    };
    let mem_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(mem_color))
                .title("🧠 Memory Usage")
                .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        )
        .data(mem)
        .style(Style::default().fg(mem_color))
        .max(100);
    f.render_widget(mem_sparkline, charts_layout[1]);

    let msg_sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.warning))
                .title("📡 Messages/sec")
                .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        )
        .data(msgs)
        .style(Style::default().fg(theme.warning));
    f.render_widget(msg_sparkline, charts_layout[2]);

    let events_items: Vec<ListItem> = events
        .iter()
        .take(50)
        .enumerate()
        .map(|(i, (time, msg))| {
            let time_ago = format!("{:>3}s", time.elapsed().as_secs());
            let styled_line = Line::from(vec![
                Span::styled(time_ago, Style::default().fg(theme.muted)),
                Span::styled(" │ ", Style::default().fg(theme.muted)),
                Span::styled(
                    msg,
                    Style::default().fg(if i < 5 { theme.text } else { theme.muted }),
                ),
            ]);
            ListItem::new(styled_line)
        })
        .collect();

    let events_list = List::new(events_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.primary))
            .title("📝 Recent Events")
            .title_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(events_list, main_layout[2]);
}
