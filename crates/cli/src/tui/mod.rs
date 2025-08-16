#![allow(clippy::collapsible_match, clippy::single_match)]

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use libp2p::{gossipsub, identity, mdns, ping, swarm::SwarmEvent, Multiaddr, PeerId, SwarmBuilder};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{ListState, TableState},
    Terminal,
};
use tokio::sync::mpsc;

use base64::Engine;
use common::{
    deserialize_message, AgentUpgrade, Command, OwnerKeypair, PushPackage, PushUnsigned,
    SignedManifest, Status, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, sha256_hex, sign_bytes_ed25519,
};

mod draw;
use draw::*;

const EVENTS_CAP: usize = 500;

thread_local! {
    static LAST_RESTARTS: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static LAST_PUBERR: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static LAST_FUEL: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static LAST_MEM_CUR: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static LAST_MEM_PEAK: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Overview,
    Peers,
    Deployments,
    Topology,
    Events,
    Logs,
    Ops,
}

struct PeerRow {
    last_msg_at: Instant,
    last_ping: Instant,
    agent_version: u64,
    roles: String,
    desired_components: u64,
    running_components: u64,
}

#[derive(Clone, Debug)]
struct PushWizard {
    step: usize,
    file: String,
    replicas: u32,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    tags_csv: String,
    start: bool,
}

impl Default for PushWizard {
    fn default() -> Self {
        Self {
            step: 0,
            file: String::new(),
            replicas: 1,
            memory_max_mb: 64,
            fuel: 5_000_000,
            epoch_ms: 100,
            tags_csv: String::new(),
            start: true,
        }
    }
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
    let (mut swarm, topic_cmd, topic_status, local_peer_id) = new_swarm_tui().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap(),
    )?;

    // mpsc for UI events and outgoing commands
    enum AppEvent {
        Tick,
        Key(KeyEvent),
        Gossip(Status),
        Connected(usize),
        Ping(PeerId, Duration),
        PublishError(String),
        MdnsDiscovered(Vec<(PeerId, Multiaddr)>),
        MdnsExpired(Vec<(PeerId, Multiaddr)>),
        Metrics(String),
        Logs(String),
        LogComponents(Vec<String>),
        LogTail(Vec<String>),
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();
    // dial channel to avoid moving swarm out of task
    let (dial_tx, mut dial_rx) = mpsc::unbounded_channel::<Multiaddr>();

    // Tick task
    let tx_tick = tx.clone();
    tokio::spawn(async move {
        let mut intv = tokio::time::interval(Duration::from_millis(250));
        loop {
            intv.tick().await;
            let _ = tx_tick.send(AppEvent::Tick);
        }
    });

    // Keyboard task (blocking crossterm)
    let tx_key = tx.clone();
    tokio::task::spawn_blocking(move || loop {
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let CEvent::Key(key) = event::read().unwrap_or(CEvent::Resize(0, 0)) {
                let _ = tx_key.send(AppEvent::Key(key));
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
                    match ev {
                        libp2p::mdns::Event::Discovered(list) => {
                            for (peer, _addr) in list {
                                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(
            topic_cmd.clone(),
            common::serialize_message(&common::Command::StatusQuery),
        ) {
            let _ = tx_swarm.send(AppEvent::PublishError(format!(
                "publish StatusQuery error: {e}"
            )));
        }

        let mut connected = 0usize;
        loop {
            tokio::select! {
                event = swarm.select_next_some() => {
                    match event {
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
                        SwarmEvent::Behaviour(NodeBehaviourEvent::Ping(ev)) => {
                            if let Ok(rtt) = ev.result { let _ = tx_swarm.send(AppEvent::Ping(ev.peer, rtt)); }
                        }
                        SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                            match ev {
                                mdns::Event::Discovered(list) => { let _ = tx_swarm.send(AppEvent::MdnsDiscovered(list)); }
                                mdns::Event::Expired(list) => { let _ = tx_swarm.send(AppEvent::MdnsExpired(list)); }
                            }
                        }
                        _ => {}
                    }
                }
                Some(cmd) = cmd_rx.recv() => {
                    if let Err(e) = swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(topic_cmd.clone(), common::serialize_message(&cmd))
                    {
                        let _ = tx_swarm.send(AppEvent::PublishError(format!("publish error: {e}")));
                    }
                }
                Some(addr) = dial_rx.recv() => {
                    let _ = libp2p::Swarm::dial(&mut swarm, addr);
                }
            }
        }
    });

    // App state
    let mut view = View::Overview;
    let mut nav_collapsed = false;
    let mut selected_nav = 0usize;
    let nav_items = [
        "Overview",
        "Peers",
        "Deployments",
        "Topology",
        "Events",
        "Logs",
        "Ops",
    ];

    let mut events: VecDeque<(Instant, String)> = VecDeque::with_capacity(EVENTS_CAP);
    let mut peers: BTreeMap<String, PeerRow> = BTreeMap::new();
    let mut topo: BTreeMap<String, (Option<String>, Instant)> = BTreeMap::new();
    let mut peers_table_state = TableState::default();
    let mut peer_latency: BTreeMap<String, u128> = BTreeMap::new();
    let mut cpu_hist: Vec<u64> = vec![0; 60];
    let mut mem_hist: Vec<u64> = vec![0; 60];
    let mut msg_hist: Vec<u64> = vec![0; 60];
    let _desired_hist: Vec<u64> = vec![0; 60];
    let _running_hist: Vec<u64> = vec![0; 60];
    let mut last_msg_count = 0usize;
    let mut last_sample = Instant::now();
    let sys = std::sync::Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));

    let mut overlay_msg: Option<(Instant, String)> = None;
    let mut filter_input: Option<String> = None;
    let mut log_filter: Option<String> = None;
    let mut logs_paused = false;
    let metrics_url = "http://127.0.0.1:9920/metrics".to_string();
    let logs_events_url = "http://127.0.0.1:9920/logs?component=__all__&tail=200".to_string();
    let logs_base_url = "http://127.0.0.1:9920/logs".to_string();
    let mut log_components: Vec<String> = Vec::new();
    let mut log_lines: VecDeque<String> = VecDeque::with_capacity(200);
    let mut logs_list_state = ListState::default();
    logs_list_state.select(Some(0));
    let selected_component = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let mut link_count: usize = 0;
    let mut push_wizard: Option<PushWizard> = None;

    // background fetchers for metrics and logs
    {
        let tx_m = tx.clone();
        let metrics_url = metrics_url.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut intv = tokio::time::interval(Duration::from_secs(2));
            loop {
                intv.tick().await;
                if let Ok(resp) = client.get(&metrics_url).send().await {
                    if let Ok(text) = resp.text().await {
                        let _ = tx_m.send(AppEvent::Metrics(text));
                    }
                }
            }
        });
    }
    {
        let tx_l = tx.clone();
        let logs_url = logs_events_url.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut intv = tokio::time::interval(Duration::from_secs(3));
            loop {
                intv.tick().await;
                if let Ok(resp) = client.get(&logs_url).send().await {
                    if let Ok(text) = resp.text().await {
                        let _ = tx_l.send(AppEvent::Logs(text));
                    }
                }
            }
        });
    }
    {
        let tx_l2 = tx.clone();
        let base = logs_base_url.clone();
        let selected = selected_component.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut intv = tokio::time::interval(Duration::from_secs(3));
            loop {
                intv.tick().await;
                if let Ok(resp) = client.get(&base).send().await {
                    if let Ok(text) = resp.text().await {
                        let list: Vec<String> =
                            text.lines().skip(1).map(|s| s.to_string()).collect();
                        let _ = tx_l2.send(AppEvent::LogComponents(list));
                    }
                }
                let name = { selected.lock().await.clone() };
                if !name.is_empty() {
                    let url = format!("{base}?component={name}&tail=200");
                    if let Ok(resp) = client.get(&url).send().await {
                        if let Ok(text) = resp.text().await {
                            let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
                            let _ = tx_l2.send(AppEvent::LogTail(lines));
                        }
                    }
                }
            }
        });
    }

    loop {
        // handle channel
        while let Ok(evt) = rx.try_recv() {
            match evt {
                AppEvent::Key(key) => {
                    if let Some(buf) = &mut filter_input {
                        match key.code {
                            KeyCode::Esc => { filter_input = None; push_wizard = None; }
                            KeyCode::Enter => {
                                let input_val = buf.trim().to_string();
                                if let Some(mut wiz) = push_wizard.clone() {
                                    match wiz.step {
                                        0 => { wiz.file = input_val; wiz.step = 1; overlay_msg = Some((Instant::now(), "push: replicas (default 1)".into())); filter_input = Some(String::new()); }
                                        1 => { if !input_val.is_empty() { wiz.replicas = input_val.parse().unwrap_or(1); } wiz.step = 2; overlay_msg = Some((Instant::now(), "push: memory_mb (default 64)".into())); filter_input = Some(String::new()); }
                                        2 => { if !input_val.is_empty() { wiz.memory_max_mb = input_val.parse().unwrap_or(64); } wiz.step = 3; overlay_msg = Some((Instant::now(), "push: fuel (default 5000000)".into())); filter_input = Some(String::new()); }
                                        3 => { if !input_val.is_empty() { wiz.fuel = input_val.parse().unwrap_or(5_000_000); } wiz.step = 4; overlay_msg = Some((Instant::now(), "push: epoch_ms (default 100)".into())); filter_input = Some(String::new()); }
                                        4 => { if !input_val.is_empty() { wiz.epoch_ms = input_val.parse().unwrap_or(100); } wiz.step = 5; overlay_msg = Some((Instant::now(), "push: target tags (comma-separated, optional)".into())); filter_input = Some(String::new()); }
                                        5 => { wiz.tags_csv = input_val; wiz.step = 6; overlay_msg = Some((Instant::now(), "push: start? (y/N)".into())); filter_input = Some(String::new()); }
                                        _ => {
                                            let yes = input_val.to_lowercase();
                                            wiz.start = yes == "y" || yes == "yes" || yes.is_empty();
                                            // finalize send
                                            let target_peer: Option<String> = if view == View::Peers { peers_table_state.selected().and_then(|idx| peers.keys().nth(idx).cloned()) } else { None };
                                            let tx_pub = cmd_tx.clone();
                                            let tx_evt = tx.clone();
                                            let file = wiz.file.clone();
                                            let replicas = wiz.replicas;
                                            let mem = wiz.memory_max_mb;
                                            let fuel = wiz.fuel;
                                            let epoch = wiz.epoch_ms;
                                            let tags: Vec<String> = wiz.tags_csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                                            tokio::spawn(async move {
                                                let key_path = match dirs::config_dir() { Some(mut d) => { d.push("realm"); d.push("owner.key.json"); d }, None => { let _ = tx_evt.send(AppEvent::PublishError("push: owner dir missing".into())); return; } };
                                                match tokio::fs::read(&key_path).await {
                                                    Ok(bytes) => {
                                                        if let Ok(kp) = serde_json::from_slice::<OwnerKeypair>(&bytes) {
                                                            match tokio::fs::read(&file).await {
                                                                Ok(bin) => {
                                                                    let digest = sha256_hex(&bin);
                                                                    let unsigned = PushUnsigned {
                                                                        alg: "ed25519".into(),
                                                                        owner_pub_bs58: kp.public_bs58.clone(),
                                                                        component_name: std::path::Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or("component").to_string(),
                                                                        target_peer_ids: target_peer.clone().into_iter().collect(),
                                                                        target_tags: tags,
                                                                        memory_max_mb: Some(mem),
                                                                        fuel: Some(fuel),
                                                                        epoch_ms: Some(epoch),
                                                                        replicas,
                                                                        start: wiz.start,
                                                                        binary_sha256_hex: digest,
                                                                    };
                                                                    if let Ok(unsigned_bytes) = serde_json::to_vec(&unsigned) {
                                                                        if let Ok(sig) = sign_bytes_ed25519(&kp.private_hex, &unsigned_bytes) {
                                                                            let pkg = PushPackage {
                                                                                unsigned,
                                                                                binary_b64: base64::engine::general_purpose::STANDARD.encode(&bin),
                                                                                signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
                                                                            };
                                                                            let _ = tx_pub.send(Command::PushComponent(pkg));
                                                                            let _ = tx_evt.send(AppEvent::PublishError(format!("pushed to {}", target_peer.unwrap_or_else(|| "all".into()))));
                                                                        }
                                                                    }
                                                                }
                                                                Err(_) => { let _ = tx_evt.send(AppEvent::PublishError("push: read file failed".into())); }
                                                            }
                                                        }
                                                    }
                                                    Err(_) => { let _ = tx_evt.send(AppEvent::PublishError("push: owner key missing; run 'realm init'".into())); }
                                                }
                                            });
                                            overlay_msg = Some((Instant::now(), "push: sent".into()));
                                            push_wizard = None;
                                            filter_input = None;
                                        }
                                    }
                                    push_wizard = Some(wiz);
                                    continue;
                                }
                                // original flows
                                if buf.starts_with('+') {
                                    let addr = buf.trim_start_matches('+').trim().to_string();
                                    if let Ok(ma) = addr.parse::<Multiaddr>() {
                                        let _ = dial_tx.send(ma.clone());
                                        events.push_front((Instant::now(), format!("dialing {ma}")));
                                        tokio::spawn(async move {
                                            if let Some(mut base) = dirs::data_dir() {
                                                base.push("realm-agent");
                                                let _ = tokio::fs::create_dir_all(&base).await;
                                                base.push("bootstrap.json");
                                                let mut list: Vec<String> = Vec::new();
                                                if let Ok(bytes) = tokio::fs::read(&base).await {
                                                    if let Ok(existing) = serde_json::from_slice::<Vec<String>>(&bytes) {
                                                        list = existing;
                                                    }
                                                }
                                                let s = addr;
                                                if !list.iter().any(|x| x == &s) {
                                                    list.push(s);
                                                    if let Ok(out) = serde_json::to_vec_pretty(&list) {
                                                        let _ = tokio::fs::write(&base, out).await;
                                                    }
                                                }
                                            }
                                        });
                                    } else {
                                        events.push_front((Instant::now(), format!("bad multiaddr: {addr}")));
                                    }
                                } else {
                                    log_filter = if buf.is_empty() { None } else { Some(buf.clone()) };
                                }
                                filter_input = None;
                            }
                            KeyCode::Char(c) => { buf.push(c); }
                            KeyCode::Backspace => { buf.pop(); }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                let cmd = Command::ApplyManifest(SignedManifest {
                                    alg: String::new(),
                                    owner_pub_bs58: String::new(),
                                    version: 0,
                                    manifest_toml: String::new(),
                                    signature_b64: String::new(),
                                });
                                let _ = cmd_tx.send(cmd);
                                overlay_msg = Some((Instant::now(), "apply manifest".to_string()));
                            }
                            KeyCode::Char('u') | KeyCode::Char('U') => {
                                let cmd = Command::UpgradeAgent(AgentUpgrade {
                                    alg: String::new(),
                                    owner_pub_bs58: String::new(),
                                    version: 0,
                                    binary_sha256_hex: String::new(),
                                    binary_b64: String::new(),
                                    signature_b64: String::new(),
                                });
                                let _ = cmd_tx.send(cmd);
                                overlay_msg = Some((Instant::now(), "upgrade agent".to_string()));
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') => {
                                let cmd = Command::Run {
                                    wasm_path: String::new(),
                                    memory_max_mb: 0,
                                    fuel: 0,
                                    epoch_ms: 0,
                                };
                                let _ = cmd_tx.send(cmd);
                                overlay_msg = Some((Instant::now(), "run".to_string()));
                            }
                            KeyCode::Char('/') => { filter_input = Some(String::new()); }
                            KeyCode::Char('P') => { push_wizard = Some(PushWizard::default()); overlay_msg = Some((Instant::now(), "push: file path".into())); filter_input = Some(String::new()); }
                            KeyCode::Char('p') => {
                                logs_paused = !logs_paused;
                                overlay_msg = Some((
                                    Instant::now(),
                                    if logs_paused { "logs paused".into() } else { "logs resumed".into() },
                                ));
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if on_key(
                                    key,
                                    &mut view,
                                    &mut nav_collapsed,
                                    &mut selected_nav,
                                    &mut peers_table_state,
                                )? {
                                    break;
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if on_key(
                                    key,
                                    &mut view,
                                    &mut nav_collapsed,
                                    &mut selected_nav,
                                    &mut peers_table_state,
                                )? {
                                    break;
                                }
                            }
                            KeyCode::PageUp => {
                                if view == View::Logs {
                                    if let Some(idx) = logs_list_state.selected() {
                                        let new = idx.saturating_sub(1);
                                        logs_list_state.select(Some(new));
                                        if let Some(name) = log_components.get(new) {
                                            let mut sel = selected_component.lock().await;
                                            *sel = name.clone();
                                        }
                                    }
                                }
                            }
                            KeyCode::PageDown => {
                                if view == View::Logs {
                                    let next = logs_list_state
                                        .selected()
                                        .unwrap_or(0)
                                        .saturating_add(1);
                                    if next < log_components.len() {
                                        logs_list_state.select(Some(next));
                                        if let Some(name) = log_components.get(next) {
                                            let mut sel = selected_component.lock().await;
                                            *sel = name.clone();
                                        }
                                    }
                                }
                            }
                            _ => {
                                if on_key(
                                    key,
                                    &mut view,
                                    &mut nav_collapsed,
                                    &mut selected_nav,
                                    &mut peers_table_state,
                                )? {
                                    break;
                                }
                            }
                        }
                    }
                }
                AppEvent::Gossip(s) => {
                    let pid = s.node_id;
                    peers
                        .entry(pid.clone())
                        .and_modify(|p| {
                            p.last_msg_at = Instant::now();
                            p.agent_version = s.agent_version;
                            p.roles = if s.tags.is_empty() { String::new() } else { s.tags.join(",") };
                            p.desired_components = s.components_desired;
                            p.running_components = s.components_running;
                        })
                        .or_insert(PeerRow {
                            last_msg_at: Instant::now(),
                            last_ping: Instant::now(),
                            agent_version: s.agent_version,
                            roles: if s.tags.is_empty() { String::new() } else { s.tags.join(",") },
                            desired_components: s.components_desired,
                            running_components: s.components_running,
                        });
                    if !logs_paused {
                        events.push_front((Instant::now(), s.msg));
                        if events.len() > EVENTS_CAP {
                            events.pop_back();
                        }
                    }
                    last_msg_count += 1;
                }
                AppEvent::Tick => {}
                AppEvent::Connected(n) => { link_count = n; }
                AppEvent::Ping(peer, rtt) => {
                    peer_latency.insert(peer.to_string(), rtt.as_millis());
                    peers
                        .entry(peer.to_string())
                        .and_modify(|p| p.last_ping = Instant::now())
                        .or_insert(PeerRow {
                            last_msg_at: Instant::now(),
                            last_ping: Instant::now(),
                            agent_version: 0,
                            roles: String::new(),
                            desired_components: 0,
                            running_components: 0,
                        });
                }
                AppEvent::PublishError(msg) => {
                    let overlay_text = msg.clone();
                    let event_msg = if logs_paused {
                        format!("{msg} [missed while paused]")
                    } else {
                        msg
                    };
                    events.push_front((Instant::now(), event_msg));
                    if events.len() > EVENTS_CAP {
                        events.pop_back();
                    }
                    overlay_msg = Some((Instant::now(), overlay_text));
                }
                AppEvent::MdnsDiscovered(list) => {
                    for (peer, addr) in list {
                        topo.insert(peer.to_string(), (Some(addr.to_string()), Instant::now()));
                    }
                }
                AppEvent::MdnsExpired(list) => {
                    for (peer, _addr) in list {
                        // Keep entry but update last seen to now, indicating recent expiry
                        topo.entry(peer.to_string())
                            .and_modify(|e| e.1 = Instant::now())
                            .or_insert((None, Instant::now()));
                    }
                }
                AppEvent::Metrics(text) => {
                    // Parse a few gauges/counters for Overview
                    fn parse_metric(text: &str, key: &str) -> Option<u64> {
                        for line in text.lines() {
                            if let Some(rest) = line.strip_prefix(key) {
                                let v = rest.trim().split_whitespace().last()?;
                                if let Ok(n) = v.parse::<u64>() { return Some(n); }
                            }
                        }
                        None
                    }
                    let msgs = parse_metric(&text, "agent_msgs_per_sec").unwrap_or(0);
                    let restarts = parse_metric(&text, "agent_restarts_total").unwrap_or(0);
                    let puberr = parse_metric(&text, "agent_status_publish_errors_total").unwrap_or(0);
                    let fuel = parse_metric(&text, "agent_fuel_used_total").unwrap_or(0);
                    let mem_cur = parse_metric(&text, "agent_mem_current_bytes").unwrap_or(0);
                    let mem_peak = parse_metric(&text, "agent_mem_peak_bytes").unwrap_or(0);
                    // update sparkline and store latest for draw
                    msg_hist.rotate_left(1);
                    msg_hist[59] = msgs;
                    // stash as a synthetic event to keep simple (could be stored in dedicated vars)
                    if !logs_paused {
                        events.push_front((Instant::now(), format!("metrics msgs/s={msgs} restarts={restarts} puberr={puberr}")));
                        if events.len() > EVENTS_CAP { events.pop_back(); }
                    }
                    // store latest in place via closures capturing outer mut refs
                    LAST_RESTARTS.with(|c| c.set(restarts));
                    LAST_PUBERR.with(|c| c.set(puberr));
                    LAST_FUEL.with(|c| c.set(fuel));
                    LAST_MEM_CUR.with(|c| c.set(mem_cur));
                    LAST_MEM_PEAK.with(|c| c.set(mem_peak));
                }
                AppEvent::Logs(text) => {
                    if !logs_paused {
                        if let Some(last) = text.lines().last() {
                            events.push_front((Instant::now(), format!("logs: {last}")));
                            if events.len() > EVENTS_CAP {
                                events.pop_back();
                            }
                        }
                    }
                }
                AppEvent::LogComponents(list) => {
                    log_components = list;
                    if log_components.is_empty() {
                        logs_list_state.select(None);
                        let mut sel = selected_component.lock().await;
                        sel.clear();
                    } else {
                        let idx = logs_list_state
                            .selected()
                            .unwrap_or(0)
                            .min(log_components.len() - 1);
                        logs_list_state.select(Some(idx));
                        if let Some(name) = log_components.get(idx) {
                            let mut sel = selected_component.lock().await;
                            *sel = name.clone();
                        }
                    }
                }
                AppEvent::LogTail(lines) => {
                    log_lines.clear();
                    for l in lines {
                        log_lines.push_back(l);
                    }
                }
            }
        }

        // sample every second
        if last_sample.elapsed() >= Duration::from_secs(1) {
            let mut sys_locked = sys.lock().await;
            sys_locked.refresh_all();
            let cpu = (sys_locked.global_cpu_info().cpu_usage() as u64).min(100);
            let mem = ((sys_locked.used_memory() as f64 / sys_locked.total_memory() as f64) * 100.0)
                as u64;
            cpu_hist.rotate_left(1);
            cpu_hist[59] = cpu;
            mem_hist.rotate_left(1);
            mem_hist[59] = mem;
            msg_hist.rotate_left(1);
            msg_hist[59] = last_msg_count as u64;
            // aggregate desired/running from latest peer rows if available (placeholder: sum unknown -> 0)
            // desired/running shown from latest peer rows when drawing
            last_msg_count = 0;
            last_sample = Instant::now();
        }

        if let Some((t, _)) = &overlay_msg {
            if t.elapsed() > Duration::from_secs(2) {
                overlay_msg = None;
            }
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

            draw_top(f, chunks[0], &view, peers.len(), link_count, &local_peer_id);

            let body = chunks[1];
            let left_w = if nav_collapsed || body.width < 60 {
                0
            } else {
                18
            };
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(1)])
                .split(body);

            if left_w > 0 {
                draw_nav(f, cols[0], &nav_items, selected_nav);
            }

            match view {
                View::Overview => {
                    let components_desired_total: u64 = peers.values().map(|p| p.desired_components).sum();
                    let components_running_total: u64 = peers.values().map(|p| p.running_components).sum();
                    let (restarts_total, publish_errors_total, fuel_used_total, mem_current_bytes, mem_peak_bytes) = (
                        LAST_RESTARTS.with(|c| c.get()),
                        LAST_PUBERR.with(|c| c.get()),
                        LAST_FUEL.with(|c| c.get()),
                        LAST_MEM_CUR.with(|c| c.get()),
                        LAST_MEM_PEAK.with(|c| c.get()),
                    );
                    draw_overview(
                        f,
                        cols[1],
                        &cpu_hist,
                        &mem_hist,
                        &msg_hist,
                        peers.len(),
                        &events,
                        components_desired_total,
                        components_running_total,
                        restarts_total,
                        publish_errors_total,
                        fuel_used_total,
                        mem_current_bytes,
                        mem_peak_bytes,
                    )
                }
                View::Peers => {
                    draw_peers(f, cols[1], &peers, &peer_latency, &mut peers_table_state)
                }
                View::Deployments => draw_placeholder(f, cols[1], "Deployments: no data yet"),
                View::Topology => draw_topology(f, cols[1], &topo),
                View::Events => draw_logs(f, cols[1], &events, log_filter.as_deref(), logs_paused),
                View::Logs => draw_component_logs(
                    f,
                    cols[1],
                    &log_components,
                    &mut logs_list_state,
                    &log_lines,
                ),
                View::Ops => {
                    draw_placeholder(f, cols[1], "Ops: use keybinds A/U/W to perform actions")
                }
            }

            draw_footer(f, chunks[2]);

            if let Some((_, msg)) = &overlay_msg {
                draw_overlay(f, area, msg);
            }
            if let Some(buf) = &filter_input {
                draw_overlay(f, area, &format!("/{buf}"));
            }
        })?;
    }

    // never reached
}

fn on_key(
    key: KeyEvent,
    view: &mut View,
    nav_collapsed: &mut bool,
    selected_nav: &mut usize,
    _peers_table_state: &mut TableState,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Char('q') => {
            // leave terminal
            disable_raw_mode()?;
            let mut stdout = std::io::stdout();
            execute!(stdout, LeaveAlternateScreen)?;
            return Ok(true);
        }
        KeyCode::Char('c') => {
            *nav_collapsed = !*nav_collapsed;
        }
        KeyCode::Char('1') => {
            *view = View::Overview;
            *selected_nav = 0;
        }
        KeyCode::Char('2') => {
            *view = View::Peers;
            *selected_nav = 1;
        }
        KeyCode::Char('3') => {
            *view = View::Deployments;
            *selected_nav = 2;
        }
        KeyCode::Char('4') => {
            *view = View::Topology;
            *selected_nav = 3;
        }
        KeyCode::Char('5') => {
            *view = View::Events;
            *selected_nav = 4;
        }
        KeyCode::Char('6') => {
            *view = View::Logs;
            *selected_nav = 5;
        }
        KeyCode::Char('7') => {
            *view = View::Ops;
            *selected_nav = 6;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if *selected_nav > 0 {
                *selected_nav -= 1;
            }
            *view = match *selected_nav {
                0 => View::Overview,
                1 => View::Peers,
                2 => View::Deployments,
                3 => View::Topology,
                4 => View::Events,
                5 => View::Logs,
                _ => View::Ops,
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if *selected_nav < 6 {
                *selected_nav += 1;
            }
            *view = match *selected_nav {
                0 => View::Overview,
                1 => View::Peers,
                2 => View::Deployments,
                3 => View::Topology,
                4 => View::Events,
                5 => View::Logs,
                _ => View::Ops,
            };
        }
        _ => {}
    }
    Ok(false)
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
    ping: ping::Behaviour,
}

async fn new_swarm_tui() -> anyhow::Result<(
    libp2p::Swarm<NodeBehaviour>,
    gossipsub::IdentTopic,
    gossipsub::IdentTopic,
    PeerId,
)> {
    let id_keys = identity::Keypair::generate_ed25519();
    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;
    let mdns_beh =
        mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;
    let ping_beh = ping::Behaviour::new(ping::Config::new());
    let behaviour = NodeBehaviour {
        gossipsub,
        mdns: mdns_beh,
        ping: ping_beh,
    };
    let local_peer_id = PeerId::from(id_keys.public());
    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();
    Ok((swarm, topic_cmd, topic_status, local_peer_id))
}
