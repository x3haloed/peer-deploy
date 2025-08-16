use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use libp2p::{gossipsub, mdns, swarm::SwarmEvent, Multiaddr, PeerId, SwarmBuilder, identify, identity, kad};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Sparkline, Table, TableState,
    },
    Terminal,
};
use tokio::sync::mpsc;

use common::{deserialize_message, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View { Overview, Peers, Deployments, Topology, Logs, Ops }

struct PeerRow {
    last_msg_at: Instant,
    agent_version: u64,
    alias: String,
    roles: String,
}

pub async fn run_tui() -> anyhow::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Networking: create swarm and subscribe to topics
    let (mut swarm, topic_cmd, topic_status) = new_swarm_tui().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap())?;

    // mpsc for UI events
    enum AppEvent { Tick, Key(KeyEvent), Gossip(Status), Connected(usize) }
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();

    // Tick task
    let tx_tick = tx.clone();
    tokio::spawn(async move {
        let mut intv = tokio::time::interval(Duration::from_millis(250));
        loop { intv.tick().await; let _ = tx_tick.send(AppEvent::Tick); }
    });

    // Keyboard task (blocking crossterm)
    let tx_key = tx.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                if let CEvent::Key(key) = event::read().unwrap_or(CEvent::Resize(0,0)) {
                    let _ = tx_key.send(AppEvent::Key(key));
                }
            }
        }
    });

    // Swarm event pump
    let tx_swarm = tx.clone();
    tokio::spawn(async move {
        // brief mdns warmup
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(600) {
            if let Some(event) = swarm.next().await {
                if let SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) = event {
                    match ev { libp2p::mdns::Event::Discovered(list) => {
                        for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer); }
                    }, _ => {} }
                }
            }
        }

        let _ = swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic_cmd.clone(), common::serialize_message(&common::Command::StatusQuery));

        let mut connected = 0usize;
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                    if let gossipsub::Event::Message { message, .. } = ev {
                        if message.topic == topic_status.hash() {
                            if let Ok(status) = deserialize_message::<Status>(&message.data) {
                                let _ = tx_swarm.send(AppEvent::Gossip(status));
                            }
                        }
                    }
                }
                SwarmEvent::NewListenAddr { .. } => {}
                SwarmEvent::ConnectionEstablished { .. } => { connected = connected.saturating_add(1); let _ = tx_swarm.send(AppEvent::Connected(connected)); }
                SwarmEvent::ConnectionClosed { .. } => { connected = connected.saturating_sub(1); let _ = tx_swarm.send(AppEvent::Connected(connected)); }
                _ => {}
            }
        }
    });

    // App state
    let mut view = View::Overview;
    let mut nav_collapsed = false;
    let mut selected_nav = 0usize;
    let nav_items = ["Overview", "Peers", "Deployments", "Topology", "Logs", "Ops"];

    let mut events: VecDeque<(Instant, String)> = VecDeque::with_capacity(512);
    let mut peers: BTreeMap<String, PeerRow> = BTreeMap::new();
    let mut peers_table_state = TableState::default();
    let mut cpu_hist: Vec<u64> = vec![0; 60];
    let mut mem_hist: Vec<u64> = vec![0; 60];
    let mut msg_hist: Vec<u64> = vec![0; 60];
    let mut last_msg_count = 0usize;
    let mut last_sample = Instant::now();
    let sys = std::sync::Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));

    loop {
        // handle channel
        while let Ok(evt) = rx.try_recv() {
            match evt {
                AppEvent::Key(key) => {
                    if on_key(key, &mut view, &mut nav_collapsed, &mut selected_nav, &mut peers_table_state)? { break; }
                }
                AppEvent::Gossip(s) => {
                    let pid = s.node_id;
                    peers.entry(pid.clone()).and_modify(|p| { p.last_msg_at = Instant::now(); }).or_insert(PeerRow { last_msg_at: Instant::now(), agent_version: 0, alias: String::new(), roles: String::new() });
                    events.push_front((Instant::now(), s.msg));
                    if events.len() > 500 { events.pop_back(); }
                    last_msg_count += 1;
                }
                AppEvent::Tick => {}
                AppEvent::Connected(_) => {}
            }
        }

        // sample every second
        if last_sample.elapsed() >= Duration::from_secs(1) {
            let mut sys_locked = sys.lock().await;
            sys_locked.refresh_all();
            let cpu = (sys_locked.global_cpu_info().cpu_usage() as u64).min(100);
            let mem = ((sys_locked.used_memory() as f64 / sys_locked.total_memory() as f64) * 100.0) as u64;
            cpu_hist.rotate_left(1); cpu_hist[59] = cpu;
            mem_hist.rotate_left(1); mem_hist[59] = mem;
            msg_hist.rotate_left(1); msg_hist[59] = last_msg_count as u64; last_msg_count = 0;
            last_sample = Instant::now();
        }

        // draw
        terminal.draw(|f| {
            let area = f.size();
            let (top_h, footer_h) = (1, 1);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(top_h),
                    Constraint::Min(1),
                    Constraint::Length(footer_h),
                ])
                .split(area);

            draw_top(f, chunks[0], &view, peers.len());

            let body = chunks[1];
            let left_w = if nav_collapsed || body.width < 60 { 0 } else { 18 };
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(1)])
                .split(body);

            if left_w > 0 { draw_nav(f, cols[0], &nav_items, selected_nav); }

            match view {
                View::Overview => draw_overview(f, cols[1], &cpu_hist, &mem_hist, &msg_hist, peers.len(), &events),
                View::Peers => draw_peers(f, cols[1], &peers, &mut peers_table_state),
                View::Deployments => draw_placeholder(f, cols[1], "Deployments: no data yet"),
                View::Topology => draw_placeholder(f, cols[1], "Topology: discovery via mDNS/bootstrap only"),
                View::Logs => draw_logs(f, cols[1], &events),
                View::Ops => draw_placeholder(f, cols[1], "Ops: use keybinds A/U/W to perform actions"),
            }

            draw_footer(f, chunks[2]);
        })?;
    }

    // never reached
}

fn draw_top(f: &mut ratatui::Frame<'_>, area: Rect, view: &View, peer_count: usize) {
    let title = format!(" realm | {} | peers:{} ", match view { View::Overview=>"overview", View::Peers=>"peers", View::Deployments=>"deployments", View::Topology=>"topology", View::Logs=>"logs", View::Ops=>"ops" }, peer_count);
    let block = Block::default().borders(Borders::BOTTOM).title(Span::styled(title, Style::default().fg(Color::Cyan)));
    let p = Paragraph::new("").block(block);
    f.render_widget(p, area);
}

fn draw_nav(f: &mut ratatui::Frame<'_>, area: Rect, items: &[&str], selected: usize) {
    let list = List::new(
        items.iter().enumerate().map(|(i, s)| {
            let style = if i==selected { Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Gray) };
            ListItem::new(Span::styled(*s, style))
        }).collect::<Vec<_>>()
    ).block(Block::default().borders(Borders::RIGHT));
    f.render_widget(list, area);
}

fn draw_footer(f: &mut ratatui::Frame<'_>, area: Rect) {
    let help = "q quit  ↑/↓ or j/k tabs  1..6 jump  c collapse  A apply  U upgrade  W run  / filter";
    let p = Paragraph::new(Line::from(help)).alignment(Alignment::Center).style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, area);
}

fn draw_overview(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    cpu: &Vec<u64>,
    mem: &Vec<u64>,
    msgs: &Vec<u64>,
    peer_count: usize,
    events: &VecDeque<(Instant, String)>,
) {
    let rows = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(4), Constraint::Min(1)]).split(area);

    let tiles = Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Length(22), Constraint::Length(18), Constraint::Length(18), Constraint::Min(1)])
        .split(rows[0]);

    let tile_style = Style::default().fg(Color::Gray);
    let t1 = Paragraph::new(format!("Peers: {}", peer_count)).style(tile_style).block(Block::default().borders(Borders::ALL).title("Health"));
    f.render_widget(t1, tiles[0]);

    let s_cpu = Sparkline::default().data(cpu).style(Style::default().fg(Color::LightGreen)).max(100).block(Block::default().borders(Borders::ALL).title("CPU%"));
    let s_mem = Sparkline::default().data(mem).style(Style::default().fg(Color::LightMagenta)).max(100).block(Block::default().borders(Borders::ALL).title("MEM%"));
    let s_msg = Sparkline::default().data(msgs).style(Style::default().fg(Color::Yellow)).block(Block::default().borders(Borders::ALL).title("msgs/s"));
    let spark_row = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Min(10), Constraint::Min(10), Constraint::Min(10)]).split(rows[1]);
    f.render_widget(s_cpu, spark_row[0]);
    f.render_widget(s_mem, spark_row[1]);
    f.render_widget(s_msg, spark_row[2]);

    let list_items: Vec<ListItem> = events.iter().take(50).map(|(_, s)| ListItem::new(s.clone())).collect();
    let list = List::new(list_items).block(Block::default().borders(Borders::ALL).title("Recent events"));
    f.render_widget(list, rows[2]);
}

fn draw_peers(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    peers: &BTreeMap<String, PeerRow>,
    state: &mut TableState,
) {
    let cols = ["Peer ID", "Agent", "Last heartbeat", "Tags"];
    let header = ratatui::widgets::Row::new(cols.iter().map(|h| Line::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
    let mut rows = Vec::new();
    for (id, p) in peers.iter() {
        let secs = p.last_msg_at.elapsed().as_secs();
        rows.push(ratatui::widgets::Row::new(vec![id.clone(), p.agent_version.to_string(), format!("{}s", secs), p.roles.clone()]));
    }
    let table = Table::new(rows, [Constraint::Percentage(50), Constraint::Length(8), Constraint::Length(14), Constraint::Percentage(30)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Peers"))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));
    f.render_stateful_widget(table, area, state);
}

fn draw_logs(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    events: &VecDeque<(Instant, String)>,
) {
    let items: Vec<ListItem> = events.iter().map(|(t, s)| ListItem::new(format!("{:>4}s | {}", t.elapsed().as_secs(), s))).collect();
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(list, area);
}

fn draw_placeholder(f: &mut ratatui::Frame<'_>, area: Rect, text: &str) {
    let p = Paragraph::new(text).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn on_key(key: KeyEvent, view: &mut View, nav_collapsed: &mut bool, selected_nav: &mut usize, _peers_table_state: &mut TableState) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Char('q') => {
            // leave terminal
            disable_raw_mode()?;
            let mut stdout = std::io::stdout();
            execute!(stdout, LeaveAlternateScreen)?;
            return Ok(true);
        }
        KeyCode::Char('c') => { *nav_collapsed = !*nav_collapsed; }
        KeyCode::Char('1') => { *view = View::Overview; *selected_nav = 0; }
        KeyCode::Char('2') => { *view = View::Peers; *selected_nav = 1; }
        KeyCode::Char('3') => { *view = View::Deployments; *selected_nav = 2; }
        KeyCode::Char('4') => { *view = View::Topology; *selected_nav = 3; }
        KeyCode::Char('5') => { *view = View::Logs; *selected_nav = 4; }
        KeyCode::Char('6') => { *view = View::Ops; *selected_nav = 5; }
        KeyCode::Up | KeyCode::Char('k') => {
            if *selected_nav > 0 { *selected_nav -= 1; }
            *view = match *selected_nav { 0=>View::Overview,1=>View::Peers,2=>View::Deployments,3=>View::Topology,4=>View::Logs,_=>View::Ops };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if *selected_nav < 5 { *selected_nav += 1; }
            *view = match *selected_nav { 0=>View::Overview,1=>View::Peers,2=>View::Deployments,3=>View::Topology,4=>View::Logs,_=>View::Ops };
        }
        _ => {}
    }
    Ok(false)
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

async fn new_swarm_tui() -> anyhow::Result<(libp2p::Swarm<NodeBehaviour>, gossipsub::IdentTopic, gossipsub::IdentTopic)> {
    let id_keys = identity::Keypair::generate_ed25519();
    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    ).map_err(|e| anyhow::anyhow!(e))?;
    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;
    let mdns_beh = mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;
    let behaviour = NodeBehaviour { gossipsub, mdns: mdns_beh };
    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();
    Ok((swarm, topic_cmd, topic_status))
}


