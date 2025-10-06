use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Resolve the agent data directory in a platform-appropriate location.
pub fn agent_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or(std::env::temp_dir())
        .join("realm-agent")
}

fn trusted_owner_path() -> PathBuf {
    agent_data_dir().join("owner.pub")
}

fn state_path() -> PathBuf {
    agent_data_dir().join("state.json")
}

fn desired_manifest_path() -> PathBuf {
    agent_data_dir().join("desired_manifest.toml")
}

fn bootstrap_path() -> PathBuf {
    agent_data_dir().join("bootstrap.json")
}

fn listen_port_path() -> PathBuf {
    agent_data_dir().join("listen_port")
}

fn listen_port_tcp_path() -> PathBuf {
    agent_data_dir().join("listen_port_tcp")
}

pub fn load_trusted_owner() -> Option<String> {
    fs::read_to_string(trusted_owner_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_trusted_owner(pub_bs58: &str) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(trusted_owner_path(), pub_bs58.as_bytes());
}

/// Load a persisted UDP listen port for QUIC if present.
pub fn load_listen_port() -> Option<u16> {
    if let Ok(s) = fs::read_to_string(listen_port_path()) {
        if let Ok(p) = s.trim().parse::<u16>() {
            return Some(p);
        }
    }
    None
}

/// Persist the UDP listen port so the agent can reuse it across restarts.
pub fn save_listen_port(port: u16) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(listen_port_path(), port.to_string().as_bytes());
}

/// Load a persisted TCP listen port if present.
pub fn load_listen_port_tcp() -> Option<u16> {
    if let Ok(s) = fs::read_to_string(listen_port_tcp_path()) {
        if let Ok(p) = s.trim().parse::<u16>() {
            return Some(p);
        }
    }
    None
}

/// Persist the TCP listen port for stable restarts.
pub fn save_listen_port_tcp(port: u16) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(listen_port_tcp_path(), port.to_string().as_bytes());
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentState {
    #[serde(default, rename = "last_version")]
    pub manifest_version: u64,
    #[serde(default)]
    pub agent_version: u64,
    #[serde(default)]
    pub previous_agent_version: u64,
    /// Optional human-friendly aliases and notes per known node
    #[serde(default)]
    pub node_annotations: std::collections::BTreeMap<String, NodeAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeAnnotation {
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub fn load_state() -> AgentState {
    if let Ok(bytes) = fs::read(state_path()) {
        if let Ok(s) = serde_json::from_slice::<AgentState>(&bytes) {
            return s;
        }
    }
    AgentState::default()
}

pub fn save_state(state: &AgentState) {
    let _ = fs::create_dir_all(agent_data_dir());
    if let Ok(bytes) = serde_json::to_vec(state) {
        let _ = fs::write(state_path(), bytes);
    }
}

/// Load desired manifest TOML if present.
pub fn load_desired_manifest() -> Option<String> {
    fs::read_to_string(desired_manifest_path()).ok()
}

/// Persist desired manifest TOML for reconciliation.
pub fn save_desired_manifest(toml: &str) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(desired_manifest_path(), toml.as_bytes());
}

/// Update the persistent manifest with a new component from PushComponent.
/// This allows PushComponent commands to be restored on agent restart.
pub fn update_persistent_manifest_with_component(
    component_name: &str,
    spec: common::ComponentSpec,
) {
    let mut manifest = if let Some(toml_str) = load_desired_manifest() {
        // Try to parse existing manifest
        toml::from_str::<common::Manifest>(&toml_str).unwrap_or_else(|_| {
            tracing::warn!("Failed to parse existing manifest, creating new one");
            common::Manifest {
                components: std::collections::BTreeMap::new(),
            }
        })
    } else {
        // Create new manifest
        common::Manifest {
            components: std::collections::BTreeMap::new(),
        }
    };

    // Add or update the component
    manifest.components.insert(component_name.to_string(), spec);

    // Serialize and save
    if let Ok(toml_str) = toml::to_string(&manifest) {
        save_desired_manifest(&toml_str);
        tracing::info!(component=%component_name, "Updated persistent manifest with component");
    } else {
        tracing::warn!(component=%component_name, "Failed to serialize manifest");
    }
}

/// Load bootstrap multiaddrs if present.
pub fn load_bootstrap_addrs() -> Vec<String> {
    if let Ok(bytes) = fs::read(bootstrap_path()) {
        if let Ok(list) = serde_json::from_slice::<Vec<String>>(&bytes) {
            return list;
        }
    }
    Vec::new()
}

// Implement persistent known-peers store
fn peers_path() -> PathBuf {
    agent_data_dir().join("peers.json")
}

pub fn load_known_peers() -> Vec<String> {
    if let Ok(bytes) = fs::read(peers_path()) {
        if let Ok(list) = serde_json::from_slice::<Vec<String>>(&bytes) {
            return list;
        }
    }
    Vec::new()
}

pub fn save_known_peers(peers: &[String]) {
    let _ = fs::create_dir_all(agent_data_dir());
    if let Ok(bytes) = serde_json::to_vec(peers) {
        let _ = fs::write(peers_path(), &bytes);
    }
}

pub fn add_known_peer(addr: &str) {
    let mut peers = load_known_peers();
    if !peers.contains(&addr.to_string()) {
        peers.push(addr.to_string());
        save_known_peers(&peers);
    }
}
