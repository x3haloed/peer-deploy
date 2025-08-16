use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{THEME_MUTED, THEME_PRIMARY, THEME_SURFACE};

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
