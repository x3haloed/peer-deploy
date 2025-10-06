use axum::{extract::State, response::IntoResponse, Json};
use serde::Serialize;

use super::types::WebState;

#[derive(Serialize)]
struct DeployEvent {
    component: String,
    digest: Option<String>,
    timestamp: u64,
}

pub async fn api_deploy_history(State(state): State<WebState>) -> impl IntoResponse {
    // Synthesize history from recent logs; logs are stored as "<unix> | <message>"
    let logs = state.logs.lock().await;
    let mut out: Vec<DeployEvent> = Vec::new();
    for (_k, entries) in logs.iter() {
        for line in entries.iter().rev().take(200) {
            if let Some((ts_str, msg)) = line.split_once('|') {
                let msg = msg.trim();
                if msg.contains("deployed") {
                    let component =
                        parse_after(msg, "component '").unwrap_or_else(|| "component".to_string());
                    let ts = ts_str.trim().parse::<u64>().unwrap_or(0);
                    out.push(DeployEvent {
                        component,
                        digest: None,
                        timestamp: ts,
                    });
                }
            }
        }
    }
    Json(out)
}

fn parse_after(s: &str, marker: &str) -> Option<String> {
    if let Some(i) = s.find(marker) {
        let rest = &s[i + marker.len()..];
        if let Some(end) = rest.find("'") {
            return Some(rest[..end].to_string());
        }
    }
    None
}
