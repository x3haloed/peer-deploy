#![allow(dead_code)]
use axum::{
    extract::State,
    Json,
};

use super::types::*;

pub async fn api_discover(State(state): State<WebState>) -> Json<serde_json::Value> {
    let discovery_result = perform_network_discovery().await;
    if let Ok(discovered_peers) = &discovery_result {
        for peer_info in discovered_peers {
            if let Ok(status) = parse_peer_info(peer_info) { state.update_peer_status(status); }
        }
    }
    let discovered_nodes: Vec<serde_json::Value> = {
        let peers = state.peer_status.lock().await;
        peers.iter().map(|(node_id, status)| {
            serde_json::json!({
                "node_id": node_id,
                "agent_version": status.agent_version,
                "components_running": status.components_running,
                "components_desired": status.components_desired,
                "cpu_percent": status.cpu_percent,
                "mem_percent": status.mem_percent,
                "tags": status.tags,
                "links": status.links
            })
        }).collect()
    };
    let discovery_status = match discovery_result { Ok(ref peers) => format!("Successfully discovered {} peers", peers.len()), Err(ref e) => format!("Discovery completed with errors: {}", e) };
    crate::p2p::metrics::push_log(&state.logs, "system", format!("Network discovery triggered: {}", discovery_status)).await;
    Json(serde_json::json!({
        "discovered_nodes": discovered_nodes,
        "discovery_time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
        "discovery_status": discovery_status
    }))
}

#[derive(Clone)]
struct PeerInfo { address: String, metrics: String }

async fn perform_network_discovery() -> Result<Vec<PeerInfo>, String> {
    let mut discovered_peers = Vec::new();
    let local_ports = [9090, 3030, 8080, 7070];
    for port in local_ports {
        if let Ok(peer_info) = check_agent_endpoint(&format!("127.0.0.1:{}", port)).await { discovered_peers.push(peer_info); }
    }
    let base_ip = get_local_network_base().await?;
    for host in 1..255 {
        let ip = format!("{}.{}", base_ip, host);
        for port in [9090, 3030] {
            if let Ok(peer_info) = check_agent_endpoint(&format!("{}:{}", ip, port)).await { discovered_peers.push(peer_info); }
        }
    }
    Ok(discovered_peers)
}

async fn check_agent_endpoint(address: &str) -> Result<PeerInfo, String> {
    let url = format!("http://{}/metrics", address);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_millis(500)).build().map_err(|e| format!("Client error: {}", e))?;
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            if let Ok(metrics) = response.text().await {
                if metrics.contains("agent_version") || metrics.contains("components_running") {
                    return Ok(PeerInfo { address: address.to_string(), metrics });
                }
            }
        }
        _ => {}
    }
    Err(format!("No agent found at {}", address))
}

async fn get_local_network_base() -> Result<String, String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("Failed to create socket: {}", e))?;
    socket.connect("8.8.8.8:80").map_err(|e| format!("Failed to connect: {}", e))?;
    let local_addr = socket.local_addr().map_err(|e| format!("Failed to get local addr: {}", e))?;
    let ip_string = local_addr.ip().to_string();
    let ip_parts: Vec<&str> = ip_string.split('.').collect();
    if ip_parts.len() >= 3 { Ok(format!("{}.{}.{}", ip_parts[0], ip_parts[1], ip_parts[2])) } else { Ok("192.168.1".to_string()) }
}

fn parse_peer_info(peer_info: &PeerInfo) -> Result<common::Status, String> {
    let mut agent_version = 0;
    let mut components_running = 0;
    let mut components_desired = 0;
    for line in peer_info.metrics.lines() {
        if let Some((metric_name, value_str)) = line.split_once(' ') {
            if let Ok(value) = value_str.parse::<u64>() {
                match metric_name {
                    "agent_version" => agent_version = value,
                    "components_running" => components_running = value,
                    "components_desired" => components_desired = value,
                    _ => {}
                }
            }
        }
    }
    Ok(common::Status { node_id: peer_info.address.clone(), msg: "discovered".to_string(), agent_version, components_desired, components_running, cpu_percent: 0, mem_percent: 0, tags: vec!["discovered".to_string()], drift: components_desired as i64 - components_running as i64, trusted_owner_pub_bs58: None, links: 0 })
}


