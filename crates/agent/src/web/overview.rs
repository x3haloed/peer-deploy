#![allow(dead_code)]
use axum::{
    extract::{Query, State},
    Json,
};

use super::types::*;
use super::utils::format_timestamp;

pub async fn api_status(State(state): State<WebState>) -> Json<ApiStatus> {
    use std::sync::atomic::Ordering;
    let peers = state.peer_status.lock().await;
    let node_count = peers.len() as u32;
    let component_count = state.metrics.components_running.load(Ordering::Relaxed) as u32;
    let cpu_avg = if peers.is_empty() { 0 } else { peers.values().map(|p| p.cpu_percent as u32).sum::<u32>() / node_count };
    Json(ApiStatus { nodes: node_count.max(1), components: component_count, cpu_avg })
}

pub async fn api_nodes(State(state): State<WebState>) -> Json<Vec<ApiNode>> {
    let peers = state.peer_status.lock().await;
    let mut nodes = Vec::new();
    for (node_id, status) in peers.iter() {
        nodes.push(ApiNode {
            id: node_id.clone(),
            online: true,
            roles: status.tags.clone(),
            components_running: status.components_running as u32,
            components_desired: status.components_desired as u32,
            cpu_percent: status.cpu_percent as u32,
            mem_percent: status.mem_percent as u32,
        });
    }
    if nodes.is_empty() {
        use std::sync::atomic::Ordering;
        nodes.push(ApiNode {
            id: "local-node".to_string(),
            online: true,
            roles: vec!["local".to_string()],
            components_running: state.metrics.components_running.load(Ordering::Relaxed) as u32,
            components_desired: state.metrics.components_desired.load(Ordering::Relaxed) as u32,
            cpu_percent: 0,
            mem_percent: 0,
        });
    }
    Json(nodes)
}

pub async fn api_components(State(state): State<WebState>) -> Json<Vec<ApiComponent>> {
    let desired_components = state.supervisor.get_desired_snapshot().await;
    let mut components = Vec::new();
    for (name, desired) in desired_components.iter() {
        let replicas_desired = desired.spec.replicas.unwrap_or(1);
        let memory_mb = desired.spec.memory_max_mb.unwrap_or(64);
        let replicas_running = {
            let logs_map = state.logs.lock().await;
            let has_recent_logs = logs_map.get(name).map(|logs| !logs.is_empty()).unwrap_or(false);
            if has_recent_logs { replicas_desired } else { if replicas_desired > 0 { 1 } else { 0 } }
        };
        let running = replicas_running > 0;
        let peers = state.peer_status.lock().await;
        let nodes: Vec<String> = if peers.is_empty() { vec!["local-node".to_string()] } else { peers.keys().cloned().collect() };
        components.push(ApiComponent { name: name.clone(), running, replicas_running, replicas_desired, memory_mb: memory_mb as u32, nodes });
    }
    Json(components)
}

pub async fn api_logs(State(state): State<WebState>, Query(params): Query<LogQuery>) -> Json<Vec<ApiLog>> {
    let tail = params.tail.unwrap_or(100) as usize;
    let component = params.component.unwrap_or_else(|| "__all__".to_string());
    let logs_map = state.logs.lock().await;
    let mut api_logs = Vec::new();
    if component == "__all__" {
        let mut all_logs: Vec<(u64, String, String)> = Vec::new();
        for (comp_name, log_buffer) in logs_map.iter() {
            for log_line in log_buffer.iter() {
                if let Some((timestamp_str, message)) = log_line.split_once(" | ") {
                    if let Ok(timestamp) = timestamp_str.trim().parse::<u64>() {
                        all_logs.push((timestamp, comp_name.clone(), message.trim().to_string()));
                    }
                }
            }
        }
        all_logs.sort_by_key(|(timestamp, _, _)| *timestamp);
        let start_idx = all_logs.len().saturating_sub(tail);
        for (timestamp, comp_name, message) in all_logs.into_iter().skip(start_idx) {
            api_logs.push(ApiLog { timestamp: format_timestamp(timestamp), component: comp_name, message });
        }
    } else if let Some(log_buffer) = logs_map.get(&component) {
        let start_idx = log_buffer.len().saturating_sub(tail);
        for log_line in log_buffer.iter().skip(start_idx) {
            if let Some((timestamp_str, message)) = log_line.split_once(" | ") {
                if let Ok(timestamp) = timestamp_str.trim().parse::<u64>() {
                    api_logs.push(ApiLog { timestamp: format_timestamp(timestamp), component: component.clone(), message: message.trim().to_string() });
                }
            }
        }
    }
    if api_logs.is_empty() {
        api_logs.push(ApiLog { timestamp: format_timestamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()), component: "system".to_string(), message: if component == "__all__" { "No logs available yet".to_string() } else { format!("No logs found for component '{}'", component) } });
    }
    Json(api_logs)
}

pub async fn api_log_components(State(state): State<WebState>) -> Json<Vec<String>> {
    let map = state.logs.lock().await;
    let mut out: Vec<String> = map.keys().cloned().collect();
    out.sort();
    Json(out)
}


