use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event as CEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use crossterm::style::ResetColor;
use futures::StreamExt;
use libp2p::{swarm::SwarmEvent, Multiaddr};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    Terminal,
};
use tokio::sync::mpsc;

use common::Command;

use crate::tui::draw::{
    draw_component_logs, draw_footer, draw_header_tabs, draw_logs, draw_overlay,
    draw_overview, draw_peers, draw_placeholder, draw_topology, get_theme, draw_deployments,
};
use crate::tui::events::handle_event;
use crate::tui::network::{new_swarm_tui, NodeBehaviourEvent};
use crate::tui::state::{
    AppEvent, AppState, View, LAST_FUEL, LAST_MEM_CUR, LAST_MEM_PEAK, LAST_PUBERR, LAST_RESTARTS,
};

pub async fn run_tui() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    // Reset any lingering colors (from previous runs or terminals that persist bg attr)
    {
        let mut s = std::io::stdout();
        execute!(s, ResetColor)?;
    }
    terminal.clear()?;

    let (mut swarm, topic_cmd, topic_status, local_peer_id) = new_swarm_tui().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap(),
    )?;

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (dial_tx, mut dial_rx) = mpsc::unbounded_channel::<Multiaddr>();

    // Bootstrap: auto-dial any persisted peers from the shared agent bootstrap.json
    {
        let tx_boot = tx.clone();
        let dial_boot = dial_tx.clone();
        tokio::spawn(async move {
            if let Ok(list) = crate::cmd::util::read_bootstrap().await {
                for addr in list {
                    let s = addr.trim().to_string();
                    let s_norm = if s.starts_with('/') { s } else { format!("/{}", s) };
                    match s_norm.parse::<Multiaddr>() {
                        Ok(ma) => {
                            let _ = dial_boot.send(ma);
                            let _ = tx_boot.send(AppEvent::PublishError(format!("bootstrap dial: {}", s_norm)));
                        }
                        Err(_) => {
                            let _ = tx_boot.send(AppEvent::PublishError(format!("bad bootstrap addr: {}", s_norm)));
                        }
                    }
                }
            }
        });
    }

    // Tick task
    let tx_tick = tx.clone();
    tokio::spawn(async move {
        let mut intv = tokio::time::interval(Duration::from_millis(250));
        loop {
            intv.tick().await;
            let _ = tx_tick.send(AppEvent::Tick);
        }
    });

    // Keyboard task
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
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(600) {
            if let Some(event) = swarm.next().await {
                if let SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) = event {
                    if let libp2p::mdns::Event::Discovered(list) = ev {
                        for (peer, _addr) in list {
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                        }
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
                            if let libp2p::gossipsub::Event::Message { message, .. } = ev {
                                if message.topic == topic_status.hash() {
                                    if let Ok(status) = common::deserialize_message::<common::Status>(&message.data) {
                                        let _ = tx_swarm.send(AppEvent::Gossip(status));
                                    }
                                }
                            }
                        }
                        SwarmEvent::NewListenAddr { address, .. } => {
                            let _ = tx_swarm.send(AppEvent::PublishError(format!("listener: {address}")));
                        }
                        SwarmEvent::ConnectionEstablished { .. } => { connected = connected.saturating_add(1); let _ = tx_swarm.send(AppEvent::Connected(connected)); }
                        SwarmEvent::ConnectionClosed { .. } => { connected = connected.saturating_sub(1); let _ = tx_swarm.send(AppEvent::Connected(connected)); }
                        SwarmEvent::Behaviour(NodeBehaviourEvent::Ping(ev)) => {
                            if let Ok(rtt) = ev.result { let _ = tx_swarm.send(AppEvent::Ping(ev.peer, rtt)); }
                        }
                        SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                            match ev {
                                libp2p::mdns::Event::Discovered(list) => {
                                    // Keep gossipsub aware of freshly discovered peers beyond warm-up
                                    for (peer, _addr) in &list {
                                        swarm.behaviour_mut().gossipsub.add_explicit_peer(peer);
                                    }
                                    let _ = tx_swarm.send(AppEvent::MdnsDiscovered(list));
                                }
                                libp2p::mdns::Event::Expired(list) => {
                                    for (peer, _addr) in &list {
                                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(peer);
                                    }
                                    let _ = tx_swarm.send(AppEvent::MdnsExpired(list));
                                }
                            }
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                            let _ = tx_swarm.send(AppEvent::PublishError(format!("dial error to {:?}: {}", peer_id, error)));
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

    let selected_component = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let mut app = AppState::new(
        tx.clone(),
        cmd_tx.clone(),
        dial_tx.clone(),
        selected_component.clone(),
    );
    let metrics_url = "http://127.0.0.1:9920/metrics".to_string();
    let logs_events_url = "http://127.0.0.1:9920/logs?component=__all__&tail=200".to_string();
    let logs_base_url = "http://127.0.0.1:9920/logs".to_string();

    // Background fetchers
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
        while let Ok(evt) = rx.try_recv() {
            if handle_event(&mut app, evt).await? {
                // Restore terminal state before exiting the process to ensure a clean screen.
                disable_raw_mode()?;
                let mut stdout = std::io::stdout();
                execute!(stdout, LeaveAlternateScreen)?;
                std::process::exit(0);
            }
        }

        terminal.draw(|f| {
            let area = f.size();
            // resolve theme; avoid painting a global background to prevent color fallbacks on some terminals
            let theme = get_theme(app.theme);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)])
                .split(area);
            draw_header_tabs(
                f,
                chunks[0],
                &app.view,
                app.peers.len(),
                app.link_count,
                &local_peer_id,
                &theme,
            );
            let body = chunks[1];
            match app.view {
                View::Overview => {
                    let components_desired_total: u64 = app.peers.values().map(|p| p.desired_components).sum();
                    let components_running_total: u64 = app.peers.values().map(|p| p.running_components).sum();
                    let (restarts_total, publish_errors_total, fuel_used_total, mem_current_bytes, mem_peak_bytes) = (
                        LAST_RESTARTS.with(|c| c.get()),
                        LAST_PUBERR.with(|c| c.get()),
                        LAST_FUEL.with(|c| c.get()),
                        LAST_MEM_CUR.with(|c| c.get()),
                        LAST_MEM_PEAK.with(|c| c.get()),
                    );
                    draw_overview(
                        f,
                        body,
                        &app.cpu_hist,
                        &app.mem_hist,
                        &app.msg_hist,
                        app.peers.len(),
                        &app.events,
                        components_desired_total,
                        components_running_total,
                        restarts_total,
                        publish_errors_total,
                        fuel_used_total,
                        mem_current_bytes,
                        mem_peak_bytes,
                        &theme,
                    )
                }
                View::Peers => draw_peers(f, body, &app.peers, &app.peer_latency, &mut app.peers_table_state, &theme),
                View::Deployments => draw_deployments(f, body, &app.peers, &mut app.peers_table_state, &theme),
                View::Topology => draw_topology(f, body, &app.topo, &theme),
                View::Events => draw_logs(f, body, &app.events, app.log_filter.as_deref(), app.logs_paused, &theme),
                View::Logs => draw_component_logs(f, body, &app.log_components, &mut app.logs_list_state, &app.log_lines, &theme),
                View::Ops => draw_placeholder(f, body, "⚙️ Actions Panel\n\nUse keyboard shortcuts to perform operations:\n• A - Apply manifest\n• D - Deploy component\n• U - Upgrade agent\n• I - Install tools\n• C - Connect to peer (Peers tab)", &theme),
            };
            draw_footer(f, chunks[2], &theme);
            if let Some((_, msg)) = &app.overlay_msg {
                draw_overlay(f, area, msg, &theme);
            }
            if let Some(buf) = &app.filter_input {
                draw_overlay(f, area, &format!("/{buf}"), &theme);
            }
        })?;
    }
}
