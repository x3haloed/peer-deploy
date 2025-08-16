use std::time::{Duration, Instant};

use anyhow::anyhow;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use libp2p::{
    gossipsub, mdns,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Sparkline, Table, TableState, Tabs},
    Terminal,
};
use futures::StreamExt;
use tracing::{info};
use tracing_subscriber::EnvFilter;

use common::{deserialize_message, serialize_message, Command, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, Status};

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum View {
    Overview,
    Peers,
    Deployments,
    Topology,
    Logs,
    Ops,
}

impl View {
    fn titles() -> Vec<&'static str> {
        vec!["Overview", "Peers", "Deployments", "Topology", "Logs", "Ops"]
    }
}

impl Default for View {
    fn default() -> Self { View::Overview }
}

#[derive(Default)]
struct AppState {
    view: View,
    quit: bool,
    alerts: Vec<String>,
    peers_up: u64,
    peers_total: u64,
    components_running: u64,
    components_desired: u64,
    drift_count: u64,
    last_manifest_v: u64,
    agent_v: u64,
    cpu_hist: Vec<u64>,
    mem_hist: Vec<u64>,
    msg_hist: Vec<u64>,
    events: Vec<String>,
    peers: Vec<(String, String, u64, String, String, String)>,
    peers_table_state: TableState,
}

fn new_swarm() -> anyhow::Result<(Swarm<NodeBehaviour>, gossipsub::IdentTopic, gossipsub::IdentTopic)> {
    let id_keys = libp2p::identity::Keypair::generate_ed25519();

    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    ).map_err(|e| anyhow!(e))?;

    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;

    let mdns_beh = mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;

    let behaviour = NodeBehaviour { gossipsub, mdns: mdns_beh };

    let mut swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();

    let listen: Multiaddr = "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap();
    libp2p::Swarm::listen_on(&mut swarm, listen)?;
    Ok((swarm, topic_cmd, topic_status))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    // Networking
    let (mut swarm, topic_cmd, topic_status) = new_swarm()?;

    // UI setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = AppState::default();
    app.view = View::Overview;
    app.cpu_hist = vec![0; 60];
    app.mem_hist = vec![0; 60];
    app.msg_hist = vec![0; 60];

    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(200);

    loop {
        // Draw
        terminal.draw(|f| draw_ui(f, &app))?;

        // Input or network or tick
        let timeout = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or(Duration::from_millis(0));

        // Poll UI first
        let ui_event = crossterm::event::poll(timeout).unwrap_or(false);
        if ui_event {
            if let Ok(Event::Key(key)) = event::read() {
                if handle_key(&mut app, key, &mut swarm, &topic_cmd) { break; }
            }
        }

        // Then poll once for a swarm event
        if let Some(event) = futures::future::poll_fn(|cx| swarm.poll_next_unpin(cx)).await {
            handle_swarm_event(&mut app, event, &topic_status);
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
            // decay bars
            shift_push(&mut app.cpu_hist, rand_pct() as u64);
            shift_push(&mut app.mem_hist, rand_pct() as u64);
            shift_push(&mut app.msg_hist, rand_pct() as u64);
        }

        if app.quit { break; }
    }

    // Restore terminal
    disable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_swarm_event(app: &mut AppState, event: SwarmEvent<NodeBehaviourEvent>, topic_status: &gossipsub::IdentTopic) {
    match event {
        SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
            match ev {
                mdns::Event::Discovered(list) => {
                    for (peer, _addr) in list { app.peers_up += 1; }
                }
                mdns::Event::Expired(_list) => {}
            }
        }
        SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
            if let gossipsub::Event::Message { propagation_source: _p, message, .. } = ev {
                if message.topic == topic_status.hash() {
                    if let Ok(st) = deserialize_message::<Status>(&message.data) {
                        let line = format!("{}", st.msg);
                        push_event(app, line);
                    }
                }
            }
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            push_event(app, format!("listening on {}", address));
        }
        other => {
            if cfg!(debug_assertions) { let _ = other; }
        }
    }
}

fn handle_key(app: &mut AppState, key: KeyEvent, swarm: &mut Swarm<NodeBehaviour>, topic_cmd: &gossipsub::IdentTopic) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => { app.quit = true; return true; }
        (KeyCode::Esc, _) => { app.view = View::Overview; }
        (KeyCode::Left, _) => prev_view(app),
        (KeyCode::Right, _) => next_view(app),
        (KeyCode::Char('1'), _) => app.view = View::Overview,
        (KeyCode::Char('2'), _) => app.view = View::Peers,
        (KeyCode::Char('3'), _) => app.view = View::Deployments,
        (KeyCode::Char('4'), _) => app.view = View::Topology,
        (KeyCode::Char('5'), _) => app.view = View::Logs,
        (KeyCode::Char('6'), _) => app.view = View::Ops,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            // Ask for status; first peer to respond will show up in events
            let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&Command::StatusQuery));
        }
        _ => {}
    }
    false
}

fn prev_view(app: &mut AppState) { app.view = match app.view { View::Overview => View::Ops, View::Peers => View::Overview, View::Deployments => View::Peers, View::Topology => View::Deployments, View::Logs => View::Topology, View::Ops => View::Logs } }
fn next_view(app: &mut AppState) { app.view = match app.view { View::Overview => View::Peers, View::Peers => View::Deployments, View::Deployments => View::Topology, View::Topology => View::Logs, View::Logs => View::Ops, View::Ops => View::Overview } }

fn draw_ui(f: &mut ratatui::Frame, app: &AppState) {
    let size = f.size();

    // Dark theme baseline
    let _ = f; // unused in theme, retained for future coloring

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top bar
            Constraint::Min(3),    // body
            Constraint::Length(1), // footer
        ]).split(size);

    draw_topbar(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
}

fn draw_topbar(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let titles = View::titles().into_iter().map(|t| Span::styled(t, Style::default().fg(Color::Gray))).collect::<Vec<_>>();
    let tabs = Tabs::new(titles)
        .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .select(app.view as usize);
    f.render_widget(tabs, area);
}

fn draw_body(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    match app.view {
        View::Overview => draw_overview(f, area, app),
        View::Peers => draw_peers(f, area, app),
        View::Deployments => draw_deployments(f, area, app),
        View::Topology => draw_topology(f, area, app),
        View::Logs => draw_logs(f, area, app),
        View::Ops => draw_ops(f, area, app),
    }
}

fn draw_footer(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let help = Line::from(vec![
        Span::styled("q", Style::default().fg(Color::Gray)), Span::raw(": quit  "),
        Span::styled("←/→", Style::default().fg(Color::Gray)), Span::raw(": switch view  "),
        Span::styled("Ctrl+s", Style::default().fg(Color::Gray)), Span::raw(": query status"),
    ]);
    let block = Block::default().borders(Borders::TOP);
    let p = Paragraph::new(help).block(block);
    f.render_widget(p, area);
}

fn draw_overview(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let rows = Layout::default().direction(Direction::Vertical).constraints([
        Constraint::Length(3), // health tiles
        Constraint::Length(3), // sparklines
        Constraint::Min(3),    // events feed
    ]).split(area);

    // Tiles
    let tiles = Layout::default().direction(Direction::Horizontal).constraints([
        Constraint::Percentage(20), Constraint::Percentage(20), Constraint::Percentage(20), Constraint::Percentage(20), Constraint::Percentage(20)
    ]).split(rows[0]);

    let t1 = tile(format!("Peers {}/{}", app.peers_up, app.peers_total));
    let t2 = tile(format!("Components {}/{}", app.components_running, app.components_desired));
    let t3 = tile(format!("Drift {}", app.drift_count));
    let t4 = tile(format!("Manifest v{}", app.last_manifest_v));
    let t5 = tile(format!("Agent v{}", app.agent_v));
    f.render_widget(t1, tiles[0]);
    f.render_widget(t2, tiles[1]);
    f.render_widget(t3, tiles[2]);
    f.render_widget(t4, tiles[3]);
    f.render_widget(t5, tiles[4]);

    // Sparklines
    let srow = Layout::default().direction(Direction::Horizontal).constraints([
        Constraint::Percentage(33), Constraint::Percentage(34), Constraint::Percentage(33)
    ]).split(rows[1]);
    let sp1 = Sparkline::default().block(Block::default().title("CPU").borders(Borders::ALL)).data(&app.cpu_hist).style(Style::default().fg(Color::LightGreen));
    let sp2 = Sparkline::default().block(Block::default().title("MEM").borders(Borders::ALL)).data(&app.mem_hist).style(Style::default().fg(Color::LightMagenta));
    let sp3 = Sparkline::default().block(Block::default().title("msgs/sec").borders(Borders::ALL)).data(&app.msg_hist).style(Style::default().fg(Color::LightCyan));
    f.render_widget(sp1, srow[0]);
    f.render_widget(sp2, srow[1]);
    f.render_widget(sp3, srow[2]);

    // Events
    let items: Vec<ListItem> = app.events.iter().rev().take(100).map(|e| ListItem::new(e.clone())).collect();
    let list = List::new(items).block(Block::default().title("Recent events").borders(Borders::ALL));
    f.render_widget(list, rows[2]);
}

fn tile(title: String) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))]))
        .block(Block::default().borders(Borders::ALL))
}

fn draw_peers(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let headers = ["Peer", "Alias", "Agent", "Latency", "Last HB", "Tags"];
    let rows = app.peers.iter().map(|(pid, alias, av, lat, hb, tags)| {
        ratatui::widgets::Row::new(vec![pid.clone(), alias.clone(), format!("{}", av), lat.clone(), hb.clone(), tags.clone()])
    });
    let table = Table::new(rows, [Constraint::Percentage(25), Constraint::Percentage(15), Constraint::Percentage(10), Constraint::Percentage(10), Constraint::Percentage(20), Constraint::Percentage(20)])
        .header(ratatui::widgets::Row::new(headers).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().title("Peers").borders(Borders::ALL));
    f.render_stateful_widget(table, area, &mut app.peers_table_state.clone());
}

fn draw_deployments(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let p = Paragraph::new("Deployments view (grid, diff, details) — coming next").block(Block::default().title("Deployments").borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_topology(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let p = Paragraph::new("Topology view (ASCII graph) — coming next").block(Block::default().title("Topology").borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_logs(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let p = Paragraph::new("Logs view (tail, filter, pin) — coming next").block(Block::default().title("Logs").borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_ops(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let p = Paragraph::new("Ops view (apply, restart, upgrade, run) — coming next").block(Block::default().title("Ops").borders(Borders::ALL));
    f.render_widget(p, area);
}

fn push_event(app: &mut AppState, s: String) {
    app.events.push(s);
    if app.events.len() > 500 { let _ = app.events.drain(0..app.events.len()-500); }
}

fn shift_push(vec: &mut Vec<u64>, v: u64) { if !vec.is_empty() { vec.remove(0); vec.push(v); } }

fn rand_pct() -> u8 { use std::cell::Cell; thread_local! { static SEED: Cell<u32> = Cell::new(0x12345678); }
    SEED.with(|s| { let mut x = s.get(); x ^= x << 13; x ^= x >> 17; x ^= x << 5; s.set(x); (x % 100) as u8 }) }


