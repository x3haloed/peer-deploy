use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Resolve the agent data directory in a platform-appropriate location.
pub fn agent_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or(std::env::temp_dir()).join("realm-agent")
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentState {
    #[serde(default, rename = "last_version")]
    pub manifest_version: u64,
    #[serde(default)]
    pub agent_version: u64,
    #[serde(default)]
    pub previous_agent_version: u64,
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

/// Persist desired manifest TOML for reconciliation.
pub fn save_desired_manifest(toml: &str) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(desired_manifest_path(), toml.as_bytes());
}

/// Load desired manifest TOML if present.
pub fn load_desired_manifest() -> Option<String> {
    fs::read_to_string(desired_manifest_path()).ok()
}

