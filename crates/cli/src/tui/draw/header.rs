use libp2p::PeerId;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
};

use crate::tui::state::View;

use super::{THEME_MUTED, THEME_PRIMARY, THEME_SURFACE};

pub fn draw_header_tabs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    view: &View,
    peer_count: usize,
    link_count: usize,
    local_peer_id: &PeerId,
) {
    let time = chrono::Local::now().format("%H:%M:%S");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

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
