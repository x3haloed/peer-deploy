use std::{path::Path, time::Instant};

use crate::tui::state::{
    AppEvent, AppState, InstallWizard, PeerRow, PushWizard, UpgradeWizard, View, EVENTS_CAP,
};
use base64::Engine;
use common::{
    sha256_hex, sign_bytes_ed25519, Command, OwnerKeypair, PushPackage, PushUnsigned,
    SignedManifest,
};
use crossterm::event::{KeyCode, KeyEvent};
use libp2p::Multiaddr;
use ratatui::widgets::TableState;
pub async fn handle_event(app: &mut AppState, evt: AppEvent) -> anyhow::Result<bool> {
    match evt {
        AppEvent::Key(key) => {
            if let Some(buf) = &mut app.filter_input {
                match key.code {
                    KeyCode::Esc => {
                        app.filter_input = None;
                        app.push_wizard = None;
                        app.upgrade_wizard = None;
                        app.install_wizard = None;
                    }
                    KeyCode::Enter => {
                        let input_val = buf.trim().to_string();
                        if app.install_wizard == Some(InstallWizard::AgentPath) {
                            if input_val.is_empty() {
                                app.overlay_msg =
                                    Some((Instant::now(), "🚨 Error: File path required".into()));
                                app.filter_input = Some(String::new());
                                return Ok(false);
                            }
                            let tx_evt = app.tx.clone();
                            let path = input_val.clone();
                            #[cfg(unix)]
                            tokio::spawn(async move {
                                match crate::cmd::install(Some(path), false).await {
                                    Ok(_) => {
                                        let _ = tx_evt.send(AppEvent::PublishError(
                                            "✅ Install: Agent installation completed".into(),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = tx_evt.send(AppEvent::PublishError(format!(
                                            "❌ Install: Agent failed - {e}"
                                        )));
                                    }
                                }
                            });
                            #[cfg(not(unix))]
                            {
                                let _ = app.tx.send(AppEvent::PublishError(
                                    "❌ Install: Unsupported platform".into(),
                                ));
                            }
                            app.overlay_msg = Some((
                                Instant::now(),
                                "🚀 Install: Agent installation started".into(),
                            ));
                            app.install_wizard = None;
                            app.filter_input = None;
                            return Ok(false);
                        }
                        if let Some(mut wiz) = app.push_wizard.clone() {
                            match wiz.step {
                                0 => {
                                    if input_val.is_empty() {
                                        app.overlay_msg = Some((
                                            Instant::now(),
                                            "🚨 Error: File path required".into(),
                                        ));
                                        app.filter_input = Some(String::new());
                                        return Ok(false);
                                    }
                                    wiz.file = input_val;
                                    wiz.step = 1;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "🔢 Deploy: Number of replicas (default: 1)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                1 => {
                                    if !input_val.is_empty() {
                                        wiz.replicas = input_val.parse().unwrap_or(1);
                                    }
                                    wiz.step = 2;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "💾 Deploy: Memory limit in MB (default: 64)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                2 => {
                                    if !input_val.is_empty() {
                                        wiz.memory_max_mb = input_val.parse().unwrap_or(64);
                                    }
                                    wiz.step = 3;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "⛽ Deploy: Fuel limit (default: 5000000)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                3 => {
                                    if !input_val.is_empty() {
                                        wiz.fuel = input_val.parse().unwrap_or(5_000_000);
                                    }
                                    wiz.step = 4;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "⏱️ Deploy: Epoch time in ms (default: 100)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                4 => {
                                    if !input_val.is_empty() {
                                        wiz.epoch_ms = input_val.parse().unwrap_or(100);
                                    }
                                    wiz.step = 5;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "🏷️ Deploy: Target tags (comma-separated, optional)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                5 => {
                                    wiz.tags_csv = input_val;
                                    wiz.step = 6;
                                    app.overlay_msg = Some((
                                        Instant::now(),
                                        "🚀 Deploy: Start immediately? (y/N)".into(),
                                    ));
                                    app.filter_input = Some(String::new());
                                }
                                _ => {
                                    let yes = input_val.to_lowercase();
                                    wiz.start = yes == "y" || yes == "yes" || yes.is_empty();
                                    let target_peer: Option<String> = if app.view == View::Peers {
                                        app.peers_table_state
                                            .selected()
                                            .and_then(|idx| app.peers.keys().nth(idx).cloned())
                                    } else {
                                        None
                                    };
                                    let tx_pub = app.cmd_tx.clone();
                                    let tx_evt = app.tx.clone();
                                    let file = wiz.file.clone();
                                    let replicas = wiz.replicas;
                                    let mem = wiz.memory_max_mb;
                                    let fuel = wiz.fuel;
                                    let epoch = wiz.epoch_ms;
                                    let tags: Vec<String> = wiz
                                        .tags_csv
                                        .split(',')
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    tokio::spawn(async move {
                                        let key_path = match dirs::config_dir() {
                                            Some(mut d) => {
                                                d.push("realm");
                                                d.push("owner.key.json");
                                                d
                                            }
                                            None => {
                                                let _ = tx_evt.send(AppEvent::PublishError(
                                                    "push: owner dir missing".into(),
                                                ));
                                                return;
                                            }
                                        };
                                        match tokio::fs::read(&key_path).await {
                                            Ok(bytes) => {
                                                if let Ok(kp) =
                                                    serde_json::from_slice::<OwnerKeypair>(&bytes)
                                                {
                                                    match tokio::fs::read(&file).await {
                                                        Ok(bin) => {
                                                            let digest = sha256_hex(&bin);
                                                            let unsigned = PushUnsigned {
                                                                alg: "ed25519".into(),
                                                                owner_pub_bs58: kp
                                                                    .public_bs58
                                                                    .clone(),
                                                                component_name: Path::new(&file)
                                                                    .file_stem()
                                                                    .and_then(|s| s.to_str())
                                                                    .unwrap_or("component")
                                                                    .to_string(),
                                                                target_peer_ids: target_peer
                                                                    .clone()
                                                                    .into_iter()
                                                                    .collect(),
                                                                target_tags: tags,
                                                                memory_max_mb: Some(mem),
                                                                fuel: Some(fuel),
                                                                epoch_ms: Some(epoch),
                                                                replicas,
                                                                start: wiz.start,
                                                                binary_sha256_hex: digest,
                                                                mounts: None,
                                                            };
                                                            if let Ok(unsigned_bytes) =
                                                                serde_json::to_vec(&unsigned)
                                                            {
                                                                if let Ok(sig) = sign_bytes_ed25519(
                                                                    &kp.private_hex,
                                                                    &unsigned_bytes,
                                                                ) {
                                                                    let pkg = PushPackage {
                                                                        unsigned,
                                                                        binary_b64: base64::engine::general_purpose::STANDARD
                                                                            .encode(&bin),
                                                                        signature_b64: base64::engine::general_purpose::STANDARD
                                                                            .encode(sig),
                                                                    };
                                                                    let cmd =
                                                                        Command::PushComponent(pkg);
                                                                    let _ = tx_pub.send(cmd);
                                                                    let _ = tx_evt.send(
                                                                        AppEvent::PublishError(
                                                                            "push: sent".into(),
                                                                        ),
                                                                    );
                                                                } else {
                                                                    let _ = tx_evt.send(
                                                                        AppEvent::PublishError(
                                                                            "push: sign error"
                                                                                .into(),
                                                                        ),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ = tx_evt.send(
                                                                AppEvent::PublishError(format!(
                                                                    "push: read binary error {e}"
                                                                )),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                let _ = tx_evt.send(AppEvent::PublishError(
                                                    format!("push: read key error {e}"),
                                                ));
                                            }
                                        }
                                    });
                                    app.push_wizard = None;
                                    app.filter_input = None;
                                }
                            }
                            app.push_wizard = Some(wiz);
                            return Ok(false);
                        }
                        if app.view == View::Events {
                            app.log_filter = if buf.is_empty() {
                                None
                            } else {
                                Some(buf.clone())
                            };
                            app.filter_input = None;
                            return Ok(false);
                        }
                        if app.view == View::Peers {
                            let addr = buf.clone();
                            if let Ok(ma) = addr.parse::<Multiaddr>() {
                                app.dial_tx.send(ma).ok();
                            } else {
                                app.events
                                    .push_front((Instant::now(), format!("bad multiaddr: {addr}")));
                            }
                            app.filter_input = None;
                            return Ok(false);
                        }
                        app.filter_input = None;
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        if app.install_wizard == Some(InstallWizard::Choose) {
                            app.install_wizard = Some(InstallWizard::AgentPath);
                            app.overlay_msg =
                                Some((Instant::now(), "install agent: file path".into()));
                            app.filter_input = Some(String::new());
                        } else {
                            let cmd = Command::ApplyManifest(SignedManifest {
                                alg: String::new(),
                                owner_pub_bs58: String::new(),
                                version: 0,
                                manifest_toml: String::new(),
                                signature_b64: String::new(),
                            });
                            let _ = app.cmd_tx.send(cmd);
                            app.overlay_msg = Some((Instant::now(), "apply manifest".to_string()));
                        }
                    }
                    KeyCode::Char('u') | KeyCode::Char('U') => {
                        app.upgrade_wizard = Some(UpgradeWizard::default());
                        app.overlay_msg =
                            Some((Instant::now(), "🔄 Upgrade: Enter agent binary path".into()));
                        app.filter_input = Some(String::new());
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        app.install_wizard = Some(InstallWizard::Choose);
                        app.overlay_msg = Some((
                            Instant::now(),
                            "🔧 Install: Choose [C]LI tool or [A]gent binary".into(),
                        ));
                        app.push_wizard = None;
                        app.upgrade_wizard = None;
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        let cmd = Command::Run {
                            wasm_path: String::new(),
                            memory_max_mb: 0,
                            fuel: 0,
                            epoch_ms: 0,
                        };
                        let _ = app.cmd_tx.send(cmd);
                        app.overlay_msg = Some((Instant::now(), "run".to_string()));
                    }
                    KeyCode::Char('/') => {
                        app.filter_input = Some(String::new());
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        app.push_wizard = Some(PushWizard::default());
                        app.overlay_msg =
                            Some((Instant::now(), "🚀 Deploy: Enter file path".into()));
                        app.filter_input = Some(String::new());
                    }
                    KeyCode::Char('p') => {
                        app.logs_paused = !app.logs_paused;
                        app.overlay_msg = Some((
                            Instant::now(),
                            if app.logs_paused {
                                "logs paused".into()
                            } else {
                                "logs resumed".into()
                            },
                        ));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if on_key(key, &mut app.view, &mut app.peers_table_state)? {
                            return Ok(true);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if on_key(key, &mut app.view, &mut app.peers_table_state)? {
                            return Ok(true);
                        }
                    }
                    KeyCode::Tab => {
                        app.view = match app.view {
                            View::Overview => View::Peers,
                            View::Peers => View::Deployments,
                            View::Deployments => View::Topology,
                            View::Topology => View::Events,
                            View::Events => View::Logs,
                            View::Logs => View::Ops,
                            View::Ops => View::Overview,
                        };
                    }
                    KeyCode::PageUp => {
                        if app.view == View::Logs {
                            if let Some(idx) = app.logs_list_state.selected() {
                                let new = idx.saturating_sub(1);
                                app.logs_list_state.select(Some(new));
                                if let Some(name) = app.log_components.get(new) {
                                    let mut sel = app.selected_component.lock().await;
                                    *sel = name.clone();
                                }
                            }
                        }
                    }
                    KeyCode::PageDown => {
                        if app.view == View::Logs {
                            let next = app
                                .logs_list_state
                                .selected()
                                .unwrap_or(0)
                                .saturating_add(1);
                            if next < app.log_components.len() {
                                app.logs_list_state.select(Some(next));
                                if let Some(name) = app.log_components.get(next) {
                                    let mut sel = app.selected_component.lock().await;
                                    *sel = name.clone();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        AppEvent::Gossip(status) => {
            app.peers.insert(
                status.node_id.clone(),
                PeerRow {
                    last_msg_at: Instant::now(),
                    last_ping: Instant::now(),
                    agent_version: status.agent_version,
                    roles: status.tags.join(","),
                    desired_components: status.components_desired,
                    running_components: status.components_running,
                },
            );
            app.events
                .push_front((Instant::now(), format!("status from {}", status.node_id)));
            while app.events.len() > EVENTS_CAP {
                app.events.pop_back();
            }
        }
        AppEvent::Connected(n) => {
            app.events
                .push_front((Instant::now(), format!("connected: {n}")));
        }
        AppEvent::Ping(peer, dur) => {
            app.peer_latency.insert(peer.to_string(), dur.as_millis());
        }
        AppEvent::PublishError(e) => {
            app.events.push_front((Instant::now(), e));
        }
        AppEvent::MdnsDiscovered(list) => {
            for (peer, addr) in list {
                app.events
                    .push_front((Instant::now(), format!("mdns discovered {peer} {addr}")));
                app.topo
                    .insert(peer.to_string(), (Some(addr.to_string()), Instant::now()));
            }
        }
        AppEvent::MdnsExpired(list) => {
            for (peer, _addr) in list {
                app.events
                    .push_front((Instant::now(), format!("mdns expired {peer}")));
            }
        }
        AppEvent::Metrics(text) => {
            if !app.logs_paused {
                parse_metrics(text, app);
            }
        }
        AppEvent::Logs(text) => {
            if !app.logs_paused {
                for line in text.lines() {
                    app.log_lines.push_front(line.to_string());
                    if app.log_lines.len() > 200 {
                        app.log_lines.pop_back();
                    }
                }
            }
        }
        AppEvent::LogComponents(list) => {
            app.log_components = list;
        }
        AppEvent::LogTail(lines) => {
            if !app.logs_paused {
                app.log_lines = lines.into_iter().collect();
            }
        }
        AppEvent::Tick => {
            if let Some((t, _)) = &app.overlay_msg {
                if t.elapsed() > std::time::Duration::from_secs(5) {
                    app.overlay_msg = None;
                }
            }
            if app.last_sample.elapsed() > std::time::Duration::from_secs(1) {
                let mut sys = app.sys.lock().await;
                sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
                sys.refresh_memory();
                // network refresh not required
                sys.refresh_processes();
                let cpu = (sys.global_cpu_info().cpu_usage() * 100.0) as u64;
                let mem = sys.used_memory();
                if app.cpu_hist.len() >= 60 {
                    app.cpu_hist.remove(0);
                }
                if app.mem_hist.len() >= 60 {
                    app.mem_hist.remove(0);
                }
                app.cpu_hist.push(cpu);
                app.mem_hist.push(mem);
                let msg_cnt = app.events.len();
                if app.msg_hist.len() >= 60 {
                    app.msg_hist.remove(0);
                }
                app.msg_hist.push((msg_cnt - app.last_msg_count) as u64);
                app.last_msg_count = msg_cnt;
                app.last_sample = Instant::now();
            }
        }
    }
    Ok(false)
}

fn parse_metrics(text: String, app: &mut AppState) {
    for line in text
        .lines()
        .filter_map(|l| l.strip_prefix("agent_age_seconds "))
    {
        app.events
            .push_front((Instant::now(), format!("agent age: {line}")));
    }
}

fn on_key(
    key: KeyEvent,
    view: &mut View,
    peers_table_state: &mut TableState,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Char('q') => {
            disable()?;
            return Ok(true);
        }
        KeyCode::Char('1') => *view = View::Overview,
        KeyCode::Char('2') => *view = View::Peers,
        KeyCode::Char('3') => *view = View::Deployments,
        KeyCode::Char('4') => *view = View::Topology,
        KeyCode::Char('5') => *view = View::Events,
        KeyCode::Char('6') => *view = View::Logs,
        KeyCode::Char('7') => *view = View::Ops,
        KeyCode::Up | KeyCode::Char('k') => {
            if *view == View::Peers {
                let current = peers_table_state.selected().unwrap_or(0);
                if current > 0 {
                    peers_table_state.select(Some(current - 1));
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if *view == View::Peers {
                let current = peers_table_state.selected().unwrap_or(0);
                peers_table_state.select(Some(current + 1));
            }
        }
        _ => {}
    }
    Ok(false)
}

fn disable() -> anyhow::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::LeaveAlternateScreen)?;
    Ok(())
}
