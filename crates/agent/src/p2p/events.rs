use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct P2PEvent {
    pub timestamp: u64,
    pub direction: String,
    pub source: String,
    pub topic: String,
    pub message: String,
}
