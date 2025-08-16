use libp2p::PeerId;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
};

use crate::tui::state::View;

use super::ThemeColors;

pub fn draw_header_tabs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    view: &View,
    peer_count: usize,
    link_count: usize,
    local_peer_id: &PeerId,
    theme: &ThemeColors,
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

    // Avoid setting a background; some terminals render unknown colors as bright red.
    let title_block = Block::default().borders(Borders::NONE);

    let title_para = Paragraph::new(status_info)
        .style(Style::default().fg(theme.muted))
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
                .border_style(Style::default().fg(theme.primary)),
        )
        .style(Style::default().fg(theme.muted))
        .highlight_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )
        .select(selected_tab);

    f.render_widget(tabs, chunks[1]);
}
