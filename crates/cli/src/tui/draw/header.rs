use libp2p::PeerId;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Tabs},
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
    let time = chrono::Local::now().format("%H:%M:%S").to_string();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let status_line = Line::from(vec![
        Span::styled(
            " realm ",
            Style::default()
                .fg(theme.background)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{} peers", peer_count),
            Style::default().fg(theme.primary),
        ),
        Span::styled(" • ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{} links", link_count),
            Style::default().fg(theme.accent),
        ),
        Span::styled(" • ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{:.8}…", local_peer_id.to_string()),
            Style::default().fg(theme.muted),
        ),
        Span::styled(" • ", Style::default().fg(theme.muted)),
        Span::styled(time, Style::default().fg(theme.muted)),
    ]);

    let title_para = Paragraph::new(status_line).block(Block::default());
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
        .block(Block::default())
        .style(Style::default().fg(theme.muted))
        .highlight_style(
            Style::default()
                .fg(theme.background)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )
        .select(selected_tab);

    f.render_widget(tabs, chunks[1]);
}
