use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use libp2p::{Multiaddr, PeerId};
use ratatui::widgets::{ListState, TableState};
use tokio::sync::{mpsc, Mutex};

use common::{Command, Status};

/// Maximum number of events to retain.
pub const EVENTS_CAP: usize = 500;

thread_local! {
    pub static LAST_RESTARTS: std::cell::Cell<u64> = std::cell::Cell::new(0);
    pub static LAST_PUBERR: std::cell::Cell<u64> = std::cell::Cell::new(0);
    pub static LAST_FUEL: std::cell::Cell<u64> = std::cell::Cell::new(0);
    pub static LAST_MEM_CUR: std::cell::Cell<u64> = std::cell::Cell::new(0);
    pub static LAST_MEM_PEAK: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Overview,
    Peers,
    Deployments,
    Topology,
    Events,
    Logs,
    Ops,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallWizard {
    Choose,
    AgentPath,
}

#[derive(Clone, Debug)]
pub struct PeerRow {
    pub last_msg_at: Instant,
    pub last_ping: Instant,
    pub agent_version: u64,
    pub roles: String,
    pub desired_components: u64,
    pub running_components: u64,
}

#[derive(Clone, Debug)]
pub struct PushWizard {
    pub step: usize,
    pub file: String,
    pub replicas: u32,
    pub memory_max_mb: u64,
    pub fuel: u64,
    pub epoch_ms: u64,
    pub tags_csv: String,
    pub start: bool,
    pub static_dir: String,
    pub route_path_prefix: String,
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
            static_dir: String::new(),
            route_path_prefix: "/".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UpgradeWizard {
    pub step: usize,
    pub file: String,
    pub version: u64,
    pub tags_csv: String,
}

impl Default for UpgradeWizard {
    fn default() -> Self {
        Self {
            step: 0,
            file: String::new(),
            version: 1,
            tags_csv: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum AppEvent {
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

pub struct AppState {
    pub theme: crate::tui::draw::ThemeKind,
    pub view: View,
    pub events: VecDeque<(Instant, String)>,
    pub peers: BTreeMap<String, PeerRow>,
    pub topo: BTreeMap<String, (Option<String>, Instant)>,
    pub peers_table_state: TableState,
    pub peer_latency: BTreeMap<String, u128>,
    pub cpu_hist: Vec<u64>,
    pub mem_hist: Vec<u64>,
    pub msg_hist: Vec<u64>,
    pub last_msg_count: usize,
    pub last_sample: Instant,
    pub sys: Arc<Mutex<sysinfo::System>>,
    pub overlay_msg: Option<(Instant, String)>,
    pub filter_input: Option<String>,
    pub log_filter: Option<String>,
    pub logs_paused: bool,
    pub log_components: Vec<String>,
    pub log_lines: VecDeque<String>,
    pub logs_list_state: ListState,
    pub selected_component: Arc<Mutex<String>>,
    pub link_count: usize,
    pub owner_pub: Option<String>,
    pub push_wizard: Option<PushWizard>,
    pub upgrade_wizard: Option<UpgradeWizard>,
    pub install_wizard: Option<InstallWizard>,
    pub tx: mpsc::UnboundedSender<AppEvent>,
    pub cmd_tx: mpsc::UnboundedSender<Command>,
    pub dial_tx: mpsc::UnboundedSender<Multiaddr>,
}

impl AppState {
    pub fn new(
        tx: mpsc::UnboundedSender<AppEvent>,
        cmd_tx: mpsc::UnboundedSender<Command>,
        dial_tx: mpsc::UnboundedSender<Multiaddr>,
        selected_component: Arc<Mutex<String>>,
    ) -> Self {
        let mut logs_list_state = ListState::default();
        logs_list_state.select(Some(0));
        Self {
            view: View::Overview,
            events: VecDeque::with_capacity(EVENTS_CAP),
            peers: BTreeMap::new(),
            topo: BTreeMap::new(),
            peers_table_state: TableState::default(),
            peer_latency: BTreeMap::new(),
            cpu_hist: vec![0; 60],
            mem_hist: vec![0; 60],
            msg_hist: vec![0; 60],
            last_msg_count: 0,
            last_sample: Instant::now(),
            sys: Arc::new(Mutex::new(sysinfo::System::new_all())),
            overlay_msg: None,
            filter_input: None,
            log_filter: None,
            logs_paused: false,
            log_components: Vec::new(),
            log_lines: VecDeque::with_capacity(200),
            logs_list_state,
            selected_component,
            link_count: 0,
            owner_pub: None,
            push_wizard: None,
            upgrade_wizard: None,
            install_wizard: None,
            theme: crate::tui::draw::ThemeKind::Dark,
            tx,
            cmd_tx,
            dial_tx,
        }
    }
}
