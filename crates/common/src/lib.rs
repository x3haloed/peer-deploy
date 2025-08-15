use serde::{Deserialize, Serialize};

pub const REALM_CMD_TOPIC: &str = "realm/cmd/v1";
pub const REALM_STATUS_TOPIC: &str = "realm/status/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Hello { from: String },
    Run { wasm_path: String, memory_max_mb: u64, fuel: u64, epoch_ms: u64 },
    StatusQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub node_id: String,
    pub msg: String,
}

pub fn serialize_message<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("serialize_message")
}

pub fn deserialize_message<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}
