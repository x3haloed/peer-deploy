use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use crate::p2p::metrics::{Metrics, SharedLogs};
use crate::supervisor::Supervisor;
use common::Status;
use crate::p2p::events::P2PEvent;  // Import P2PEvent for capturing P2P messages

// Session management types
#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub last_active: SystemTime,
    pub authenticated: bool,
}

impl Session {
    pub fn new() -> Self {
        let now = SystemTime::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            last_active: now,
            authenticated: false,
        }
    }

    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now();
        match now.duration_since(self.last_active) {
            Ok(duration) => duration > std::time::Duration::from_secs(30 * 60), // 30 minutes
            Err(_) => true,
        }
    }

    pub fn touch(&mut self) {
        self.last_active = SystemTime::now();
    }
}

// Application state
#[derive(Clone)]
pub struct WebState {
    pub sessions: Arc<Mutex<std::collections::HashMap<String, Session>>>,
    pub metrics: Arc<Metrics>,
    pub logs: SharedLogs,
    pub supervisor: Arc<Supervisor>,
    // Store peer status updates from the network
    pub peer_status: Arc<tokio::sync::Mutex<BTreeMap<String, Status>>>,
    // Store P2P events from the mesh for real-time display
    pub p2p_events: Arc<tokio::sync::Mutex<Vec<P2PEvent>>>,
}

impl WebState {
    pub fn new(metrics: Arc<Metrics>, logs: SharedLogs, supervisor: Arc<Supervisor>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            metrics,
            logs,
            supervisor,
            peer_status: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            p2p_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn update_peer_status(&self, status: Status) {
        let mut peers = self.peer_status.blocking_lock();
        peers.insert(status.node_id.clone(), status);
    }

    pub fn create_session(&self) -> String {
        let session = Session::new();
        let session_id = session.id.clone();
        
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), session);
        
        // Clean up expired sessions
        sessions.retain(|_, session| !session.is_expired());
        
        session_id
    }

    pub fn authenticate_session(&self, session_id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            if !session.is_expired() {
                session.touch();
                session.authenticated = true;
                return true;
            }
        }
        false
    }
}

// API response types
#[derive(Serialize)]
pub struct ApiStatus {
    pub nodes: u32,
    pub components: u32,
    pub cpu_avg: u32,
}

#[derive(Serialize)]
pub struct ApiNode {
    pub id: String,
    pub online: bool,
    pub roles: Vec<String>,
    pub components_running: u32,
    pub components_desired: u32,
    pub cpu_percent: u32,
    pub mem_percent: u32,
}

#[derive(Serialize)]
pub struct ApiComponent {
    pub name: String,
    pub running: bool,
    pub replicas_running: u32,
    pub replicas_desired: u32,
    pub memory_mb: u32,
    pub nodes: Vec<String>,
}

#[derive(Serialize)]
pub struct ApiLog {
    pub timestamp: String,
    pub component: String,
    pub message: String,
}

#[derive(Deserialize)]
pub struct LogQuery {
    pub tail: Option<u32>,
    pub component: Option<String>,
}

#[derive(Deserialize)]
pub struct DeployRequest {
    pub name: String,
    pub source: String,  // URL or file path
    pub sha256_hex: String,
    pub replicas: Option<u32>,
    pub memory_max_mb: Option<u64>,
    pub fuel: Option<u64>,
    pub epoch_ms: Option<u64>,
}

#[derive(Deserialize)]
pub struct JobQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

// No additional types required for package deploy; using multipart

// WebSocket types
#[derive(Serialize)]
pub struct WebSocketUpdate {
    #[serde(rename = "type")]
    pub update_type: String,
    pub data: serde_json::Value,
}
