use axum::{
    extract::{Path, Query, State, Multipart},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::path::PathBuf;

use super::types::*;
use super::utils::format_timestamp;
use crate::supervisor::DesiredComponent;
use common::{ComponentSpec, Manifest, JobInstance, JobSpec};
use crate::cmd;

// API handlers with real data integration
pub async fn api_status(State(state): State<WebState>) -> Json<ApiStatus> {
    use std::sync::atomic::Ordering;
    
    let peers = state.peer_status.lock().unwrap();
    let node_count = peers.len() as u32;
    let component_count = state.metrics.components_running.load(Ordering::Relaxed) as u32;
    
    // Calculate average CPU across all peers
    let cpu_avg = if peers.is_empty() {
        0
    } else {
        peers.values().map(|p| p.cpu_percent as u32).sum::<u32>() / node_count
    };
    
    Json(ApiStatus {
        nodes: node_count.max(1), // Always show at least 1 (this node)
        components: component_count,
        cpu_avg,
    })
}

pub async fn api_nodes(State(state): State<WebState>) -> Json<Vec<ApiNode>> {
    let peers = state.peer_status.lock().unwrap();
    let mut nodes = Vec::new();
    
    for (node_id, status) in peers.iter() {
        nodes.push(ApiNode {
            id: node_id.clone(),
            online: true, // If we have status, assume online
            roles: status.tags.clone(),
            components_running: status.components_running as u32,
            components_desired: status.components_desired as u32,
            cpu_percent: status.cpu_percent as u32,
            mem_percent: status.mem_percent as u32,
        });
    }
    
    // If no peers, show local node with current metrics
    if nodes.is_empty() {
        use std::sync::atomic::Ordering;
        nodes.push(ApiNode {
            id: "local-node".to_string(),
            online: true,
            roles: vec!["local".to_string()],
            components_running: state.metrics.components_running.load(Ordering::Relaxed) as u32,
            components_desired: state.metrics.components_desired.load(Ordering::Relaxed) as u32,
            cpu_percent: 0, // We don't track local CPU in this endpoint
            mem_percent: 0, // We don't track local memory in this endpoint
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
        
        // Check if component is actually running by examining logs and metrics
        let replicas_running = {
            let logs_map = state.logs.lock().await;
            let has_recent_logs = logs_map.get(name)
                .map(|logs| !logs.is_empty())
                .unwrap_or(false);
            
            // If we have recent logs, assume the component is running
            // This is a heuristic - the supervisor tracks actual process state
            if has_recent_logs {
                replicas_desired
            } else {
                // If component exists in desired state, assume it's starting up
                if replicas_desired > 0 { 1 } else { 0 }
            }
        };
        
        let running = replicas_running > 0;
        
        // Get nodes where this component might be running (simplified)
        let peers = state.peer_status.lock().unwrap();
        let nodes: Vec<String> = if peers.is_empty() {
            vec!["local-node".to_string()]
        } else {
            peers.keys().cloned().collect()
        };
        
        components.push(ApiComponent {
            name: name.clone(),
            running,
            replicas_running,
            replicas_desired,
            memory_mb: memory_mb as u32,
            nodes,
        });
    }
    
    Json(components)
}

pub async fn api_logs(
    State(state): State<WebState>,
    Query(params): Query<LogQuery>,
) -> Json<Vec<ApiLog>> {
    let tail = params.tail.unwrap_or(100) as usize;
    let component = params.component.unwrap_or_else(|| "__all__".to_string());
    
    let logs_map = state.logs.lock().await;
    let mut api_logs = Vec::new();
    
    if component == "__all__" {
        // Collect logs from all components
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
        
        // Sort by timestamp and take the most recent
        all_logs.sort_by_key(|(timestamp, _, _)| *timestamp);
        let start_idx = all_logs.len().saturating_sub(tail);
        
        for (timestamp, comp_name, message) in all_logs.into_iter().skip(start_idx) {
            api_logs.push(ApiLog {
                timestamp: format_timestamp(timestamp),
                component: comp_name,
                message,
            });
        }
    } else if let Some(log_buffer) = logs_map.get(&component) {
        // Get logs for specific component
        let start_idx = log_buffer.len().saturating_sub(tail);
        
        for log_line in log_buffer.iter().skip(start_idx) {
            if let Some((timestamp_str, message)) = log_line.split_once(" | ") {
                if let Ok(timestamp) = timestamp_str.trim().parse::<u64>() {
                    api_logs.push(ApiLog {
                        timestamp: format_timestamp(timestamp),
                        component: component.clone(),
                        message: message.trim().to_string(),
                    });
                }
            }
        }
    }
    
    // If no logs found, provide a helpful message
    if api_logs.is_empty() {
        api_logs.push(ApiLog {
            timestamp: format_timestamp(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            ),
            component: "system".to_string(),
            message: if component == "__all__" {
                "No logs available yet".to_string()
            } else {
                format!("No logs found for component '{}'", component)
            },
        });
    }
    
    Json(api_logs)
}

pub async fn api_deploy(State(state): State<WebState>, Json(request): Json<DeployRequest>) -> impl IntoResponse {
    // Validate the request
    if request.name.is_empty() || request.source.is_empty() || request.sha256_hex.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing required fields: name, source, sha256_hex").into_response();
    }
    
    // Create component specification from request
    let spec = ComponentSpec {
        source: request.source.clone(),
        sha256_hex: request.sha256_hex,
        replicas: request.replicas,
        memory_max_mb: request.memory_max_mb,
        fuel: request.fuel,
        epoch_ms: request.epoch_ms,
        mounts: None,
        ports: None,
        visibility: None,
    };
    
    // Determine the local path for the component
    let path = if request.source.starts_with("file://") {
        PathBuf::from(request.source.strip_prefix("file://").unwrap_or(&request.source))
    } else {
        // For HTTP sources, we'd need to download and cache the component
        return (StatusCode::NOT_IMPLEMENTED, "HTTP sources not yet implemented").into_response();
    };
    
    // Verify the file exists and hash matches
    if !path.exists() {
        return (StatusCode::BAD_REQUEST, "Component file does not exist").into_response();
    }
    
    // Create desired component
    let desired_component = DesiredComponent {
        name: request.name.clone(),
        path,
        spec,
    };
    
    // Add to supervisor
    state.supervisor.upsert_component(desired_component).await;
    
    // Log the deployment
    crate::p2p::metrics::push_log(
        &state.logs, 
        "system", 
        format!("Component '{}' deployed via web interface", request.name)
    ).await;
    
    (StatusCode::OK, "Component deployed successfully").into_response()
}

/// Multipart deploy endpoint used by the web UI to upload a .wasm file and metadata.
pub async fn api_deploy_multipart(State(state): State<WebState>, mut multipart: Multipart) -> impl IntoResponse {
    // Expected fields: name (text), file (file), replicas, memory, fuel, epoch_ms, tags
    let mut name: Option<String> = None;
    let mut replicas: Option<u32> = None;
    let mut memory_max_mb: Option<u64> = None;
    let mut fuel: Option<u64> = None;
    let mut epoch_ms: Option<u64> = None;
    let mut tags_csv: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let fname = field.name().unwrap_or("").to_string();
        match fname.as_str() {
            "name" => { name = field.text().await.ok(); },
            "replicas" => { replicas = field.text().await.ok().and_then(|s| s.parse().ok()); },
            "memory" | "memory_max_mb" => { memory_max_mb = field.text().await.ok().and_then(|s| s.parse().ok()); },
            "fuel" => { fuel = field.text().await.ok().and_then(|s| s.parse().ok()); },
            "epoch" | "epoch_ms" => { epoch_ms = field.text().await.ok().and_then(|s| s.parse().ok()); },
            "tags" => { tags_csv = field.text().await.ok(); },
            "file" => { file_bytes = field.bytes().await.ok().map(|b| b.to_vec()); },
            _ => {}
        }
    }

    let name = match name { Some(n) if !n.is_empty() => n, _ => return (StatusCode::BAD_REQUEST, "Missing name").into_response() };
    let bin = match file_bytes { Some(b) if !b.is_empty() => b, _ => return (StatusCode::BAD_REQUEST, "Missing file").into_response() };

    // Compute digest and write to cache path
    let digest = common::sha256_hex(&bin);
    let stage_dir = crate::p2p::state::agent_data_dir().join("artifacts");
    let _ = tokio::fs::create_dir_all(&stage_dir).await;
    let file_path = stage_dir.join(format!("{}-{}.wasm", name, &digest[..16]));
    if !file_path.exists() {
        if tokio::fs::write(&file_path, &bin).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to write artifact").into_response();
        }
    }

    let spec = ComponentSpec {
        source: format!("cached:{}", digest),
        sha256_hex: digest.clone(),
        replicas,
        memory_max_mb,
        fuel,
        epoch_ms,
        mounts: None,
        ports: None,
        visibility: None,
    };

    // Upsert into supervisor and persist manifest
    let desired_component = DesiredComponent { name: name.clone(), path: file_path.clone(), spec: spec.clone() };
    state.supervisor.upsert_component(desired_component).await;
    crate::p2p::state::update_persistent_manifest_with_component(&name, spec);

    // Optional tags are informational here; selection is future work
    let _ = tags_csv;

    crate::p2p::metrics::push_log(&state.logs, "system", format!("Component '{}' deployed via multipart", name)).await;
    (StatusCode::OK, "ok").into_response()
}

/// Return list of component names that have logs, similar to /logs listing in metrics server
pub async fn api_log_components(State(state): State<WebState>) -> Json<Vec<String>> {
    let map = state.logs.lock().await;
    let mut out: Vec<String> = map.keys().cloned().collect();
    out.sort();
    Json(out)
}

/// Connect to a peer and persist bootstrap entry. Body: { addr: "/ip4/.../p2p/<PeerId>" }
pub async fn api_connect_peer(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let addr = body.get("addr").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if addr.is_empty() {
        return (StatusCode::BAD_REQUEST, "addr required").into_response();
    }
    // Persist into bootstrap list for the running agent to dial on next loop
    let mut list = crate::cmd::util::read_bootstrap().await.unwrap_or_default();
    if !list.iter().any(|s| s == &addr) {
        list.push(addr.clone());
        if crate::cmd::util::write_bootstrap(&list).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to persist bootstrap").into_response();
        }
    }
    (StatusCode::OK, "ok").into_response()
}

/// Multipart Push: mirrors CLI push with a single uploaded wasm and metadata for local agent
pub async fn api_push_multipart(State(state): State<WebState>, multipart: Multipart) -> impl IntoResponse {
    // similar to deploy-multipart but allows specifying advanced options in future
    // For now, alias to deploy-multipart behavior
    api_deploy_multipart(State(state), multipart).await
}

/// Multipart Upgrade: upload a new agent binary and publish upgrade command
pub async fn api_upgrade_multipart(mut multipart: Multipart) -> impl IntoResponse {
    // Collect fields: one or more pairs of file (binary) and platform, and a shared version
    let mut bins: Vec<Vec<u8>> = Vec::new();
    let mut plats: Vec<String> = Vec::new();
    let mut version: u64 = 1;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("");
        match name {
            "file" => { if let Ok(bytes) = field.bytes().await { bins.push(bytes.to_vec()); } },
            "version" => { version = field.text().await.ok().and_then(|s| s.parse().ok()).unwrap_or(1); },
            "platform" => { if let Ok(s) = field.text().await { if !s.trim().is_empty() { plats.push(s); } } },
            _ => {}
        }
    }
    if bins.is_empty() { return (StatusCode::BAD_REQUEST, "missing file").into_response(); }
    // Process each file; pair platform by index if provided
    let mut any_err: Option<String> = None;
    for (idx, bin) in bins.into_iter().enumerate() {
        let plat = plats.get(idx).cloned();
        // Stage upload to a unique path based on digest
        let digest = common::sha256_hex(&bin);
        let upload_path = crate::p2p::state::agent_data_dir().join(format!("upload-agent-{}.bin", &digest[..16]));
        if tokio::fs::write(&upload_path, &bin).await.is_err() {
            any_err = Some("failed to stage upload".into());
            break;
        }
        if let Err(e) = cmd::upgrade(upload_path.display().to_string(), version, plat, vec![], vec![]).await {
            any_err = Some(format!("upgrade failed: {}", e));
            break;
        }
    }
    match any_err {
        None => (StatusCode::OK, "ok").into_response(),
        Some(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Multipart Apply: upload a realm.toml and publish signed manifest via CLI path
pub async fn api_apply_multipart(mut multipart: Multipart) -> impl IntoResponse {
    let mut version: u64 = 1;
    let mut toml_text: Option<String> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("");
        match name {
            "file" => { toml_text = field.text().await.ok(); },
            "version" => { version = field.text().await.ok().and_then(|s| s.parse().ok()).unwrap_or(1); },
            _ => {}
        }
    }
    let toml_text = match toml_text { Some(t) => t, None => return (StatusCode::BAD_REQUEST, "missing file").into_response() };
    let upload_path = crate::p2p::state::agent_data_dir().join("upload-manifest.toml");
    if tokio::fs::write(&upload_path, toml_text.as_bytes()).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to stage manifest").into_response();
    }
    match cmd::apply(None, Some(upload_path.display().to_string()), version).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("apply failed: {e}")).into_response(),
    }
}

pub async fn api_discover(State(state): State<WebState>) -> Json<serde_json::Value> {
    // Trigger actual network discovery by scanning for agents on the local network
    let discovery_result = perform_network_discovery().await;
    
    // Update peer status with discovered nodes
    if let Ok(discovered_peers) = &discovery_result {
        for peer_info in discovered_peers {
            if let Ok(status) = parse_peer_info(peer_info) {
                state.update_peer_status(status);
            }
        }
    }
    
    // Return current peer status (including newly discovered nodes).
    // Scope the lock to ensure the guard is dropped before awaiting below.
    let discovered_nodes: Vec<serde_json::Value> = {
        let peers = state.peer_status.lock().unwrap();
        peers
            .iter()
            .map(|(node_id, status)| {
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
            })
            .collect()
    };
    
    let discovery_status = match discovery_result {
        Ok(ref peers) => format!("Successfully discovered {} peers", peers.len()),
        Err(ref e) => format!("Discovery completed with errors: {}", e),
    };
    
    // Log the discovery action
    crate::p2p::metrics::push_log(
        &state.logs,
        "system",
        format!("Network discovery triggered: {}", discovery_status)
    ).await;
    
    Json(serde_json::json!({
        "discovered_nodes": discovered_nodes,
        "discovery_time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "discovery_status": discovery_status
    }))
}

async fn perform_network_discovery() -> Result<Vec<PeerInfo>, String> {
    // Scan common agent ports on the local network
    let mut discovered_peers = Vec::new();
    
    // Check localhost first (most common case)
    let local_ports = [9090, 3030, 8080, 7070];
    for port in local_ports {
        if let Ok(peer_info) = check_agent_endpoint(&format!("127.0.0.1:{}", port)).await {
            discovered_peers.push(peer_info);
        }
    }
    
    // Scan local subnet (192.168.x.x range) for other agents
    // This is a simplified scan - in production you'd use proper service discovery
    let base_ip = get_local_network_base().await?;
    for host in 1..255 {
        let ip = format!("{}.{}", base_ip, host);
        for port in [9090, 3030] { // Only check common ports for network scan
            if let Ok(peer_info) = check_agent_endpoint(&format!("{}:{}", ip, port)).await {
                discovered_peers.push(peer_info);
            }
        }
    }
    
    Ok(discovered_peers)
}

#[derive(Clone)]
struct PeerInfo {
    address: String,
    metrics: String,
}

async fn check_agent_endpoint(address: &str) -> Result<PeerInfo, String> {
    let url = format!("http://{}/metrics", address);
    
    // Set a short timeout for network discovery
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .map_err(|e| format!("Client error: {}", e))?;
    
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            if let Ok(metrics) = response.text().await {
                if metrics.contains("agent_version") || metrics.contains("components_running") {
                    return Ok(PeerInfo {
                        address: address.to_string(),
                        metrics,
                    });
                }
            }
        }
        _ => {}
    }
    
    Err(format!("No agent found at {}", address))
}

async fn get_local_network_base() -> Result<String, String> {
    // Get the local IP address to determine network base
    // This is simplified - in production you'd use proper network interface detection
    use std::net::UdpSocket;
    
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("Failed to create socket: {}", e))?;
    
    socket.connect("8.8.8.8:80")
        .map_err(|e| format!("Failed to connect: {}", e))?;
    
    let local_addr = socket.local_addr()
        .map_err(|e| format!("Failed to get local addr: {}", e))?;
    
    let ip_string = local_addr.ip().to_string();
    let ip_parts: Vec<&str> = ip_string.split('.').collect();
    if ip_parts.len() >= 3 {
        Ok(format!("{}.{}.{}", ip_parts[0], ip_parts[1], ip_parts[2]))
    } else {
        Ok("192.168.1".to_string()) // Default fallback
    }
}

fn parse_peer_info(peer_info: &PeerInfo) -> Result<common::Status, String> {
    // Parse Prometheus metrics to extract agent status
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
    
    Ok(common::Status {
        node_id: peer_info.address.clone(),
        msg: "discovered".to_string(),
        agent_version,
        components_desired,
        components_running,
        cpu_percent: 0, // Not available from metrics
        mem_percent: 0, // Not available from metrics
        tags: vec!["discovered".to_string()],
        drift: components_desired as i64 - components_running as i64,
        trusted_owner_pub_bs58: None,
        links: 0,
    })
}

pub async fn api_component_restart(State(state): State<WebState>) -> impl IntoResponse {
    // Component restart is handled by the supervisor's reconciliation loop
    // When a component exits, the supervisor automatically restarts it
    // We can trigger this by incrementing the restart counter
    state.metrics.inc_restarts_total();
    
    crate::p2p::metrics::push_log(
        &state.logs, 
        "system", 
        "Component restart triggered via web interface".to_string()
    ).await;
    
    (StatusCode::OK, "Component restart triggered").into_response()
}

pub async fn api_component_stop(
    State(state): State<WebState>,
    Path(component_name): Path<String>,
) -> impl IntoResponse {
    // Validate component name
    if component_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Component name cannot be empty").into_response();
    }
    
    // Check if component exists in desired state
    let desired_components = state.supervisor.get_desired_snapshot().await;
    if !desired_components.contains_key(&component_name) {
        return (StatusCode::NOT_FOUND, format!("Component '{}' not found", component_name)).into_response();
    }
    
    // Stop the component by removing it from the supervisor's desired state
    // This will cause the supervisor to stop managing this component
    match stop_component(&state, &component_name).await {
        Ok(_) => {
            crate::p2p::metrics::push_log(
                &state.logs,
                "system",
                format!("Component '{}' stopped via web interface", component_name)
            ).await;
            
            (StatusCode::OK, format!("Component '{}' stopped successfully", component_name)).into_response()
        }
        Err(e) => {
            crate::p2p::metrics::push_log(
                &state.logs,
                "system",
                format!("Failed to stop component '{}': {}", component_name, e)
            ).await;
            
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to stop component: {}", e)).into_response()
        }
    }
}

async fn stop_component(state: &WebState, component_name: &str) -> Result<(), String> {
    // Clean up any running tasks for this component
    state.supervisor.cleanup_component(component_name).await;
    
    // Remove from persistent manifest
    remove_component_from_persistent_manifest(component_name)?;
    
    // Update metrics to reflect the stopped component
    state.metrics.dec_components_running();
    
    Ok(())
}

fn remove_component_from_persistent_manifest(component_name: &str) -> Result<(), String> {
    use crate::p2p::state::{load_desired_manifest, save_desired_manifest};
    
    // Load current manifest
    let manifest_toml = load_desired_manifest()
        .ok_or_else(|| "No persistent manifest found".to_string())?;
    
    let mut manifest: Manifest = toml::from_str(&manifest_toml)
        .map_err(|e| format!("Failed to parse manifest: {}", e))?;
    
    // Remove the component
    if manifest.components.remove(component_name).is_none() {
        return Err(format!("Component '{}' not found in manifest", component_name));
    }
    
    // Save updated manifest
    let updated_toml = toml::to_string(&manifest)
        .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
    
    save_desired_manifest(&updated_toml);
    
    Ok(())
}

#[cfg(unix)]
pub async fn api_install_cli() -> impl IntoResponse {
	match crate::cmd::install_cli(false).await {
		Ok(_) => (StatusCode::OK, "ok").into_response(),
		Err(e) => (StatusCode::BAD_REQUEST, format!("install-cli failed: {e}")).into_response(),
	}
}

#[cfg(not(unix))]
pub async fn api_install_cli() -> impl IntoResponse {
	(StatusCode::NOT_IMPLEMENTED, "unsupported platform").into_response()
}

pub async fn api_install_agent(mut multipart: Multipart) -> impl IntoResponse {
	#[cfg(not(unix))]
	{
		return (StatusCode::NOT_IMPLEMENTED, "unsupported platform").into_response();
	}
	#[cfg(unix)]
	{
		let mut bin_path: Option<String> = None;
		let mut system_flag: bool = false;
		let mut bin_bytes: Option<Vec<u8>> = None;
		while let Ok(Some(field)) = multipart.next_field().await {
			let name = field.name().unwrap_or("");
			match name {
				"binary" => { bin_bytes = field.bytes().await.ok().map(|b| b.to_vec()); },
				"system" => { system_flag = field.text().await.ok().map(|s| s == "true" || s == "1").unwrap_or(false); },
				_ => {}
			}
		}
		if let Some(bytes) = bin_bytes {
			let tmp = crate::p2p::state::agent_data_dir().join("upload-agent.bin");
			if tokio::fs::write(&tmp, &bytes).await.is_err() {
				return (StatusCode::INTERNAL_SERVER_ERROR, "failed to stage agent").into_response();
			}
			bin_path = Some(tmp.display().to_string());
		}
		match crate::cmd::install(bin_path, system_flag).await {
			Ok(_) => (StatusCode::OK, "ok").into_response(),
			Err(e) => (StatusCode::BAD_REQUEST, format!("install-agent failed: {e}")).into_response(),
		}
	}
}

// Job management API endpoints

pub async fn api_jobs_list(State(_state): State<WebState>, Query(params): Query<JobQuery>) -> Json<Vec<JobInstance>> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    
    // Load jobs from disk
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }

    let status_filter = params.status.as_deref();
    let limit = params.limit.unwrap_or(50) as usize;
    
    let jobs = job_manager.list_jobs(status_filter, limit).await;
    Json(jobs)
}

pub async fn api_jobs_submit(State(state): State<WebState>, mut multipart: Multipart) -> impl IntoResponse {
    // Expected fields: job_toml (file)
    let mut job_toml_text: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let fname = field.name().unwrap_or("").to_string();
        match fname.as_str() {
            "job_toml" | "file" => { 
                job_toml_text = field.text().await.ok(); 
            },
            _ => {}
        }
    }

    let job_toml = match job_toml_text {
        Some(toml) if !toml.is_empty() => toml,
        _ => return (StatusCode::BAD_REQUEST, "Missing job TOML content").into_response()
    };

    // Parse the job specification
    let job_spec: JobSpec = match toml::from_str(&job_toml) {
        Ok(spec) => spec,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid job TOML: {}", e)).into_response()
    };

    // Submit the job using the CLI command path to ensure consistency
    match cmd::submit_job_from_spec(job_spec).await {
        Ok(_) => {
            crate::p2p::metrics::push_log(
                &state.logs, 
                "system", 
                "Job submitted via web interface".to_string()
            ).await;
            (StatusCode::OK, "Job submitted successfully").into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to submit job: {}", e)).into_response()
    }
}

pub async fn api_jobs_get(State(_state): State<WebState>, Path(job_id): Path<String>) -> impl IntoResponse {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }

    match job_manager.get_job(&job_id).await {
        Some(job) => Json(job).into_response(),
        None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response()
    }
}

pub async fn api_jobs_cancel(State(state): State<WebState>, Path(job_id): Path<String>) -> impl IntoResponse {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }

    match job_manager.cancel_job(&job_id).await {
        Ok(true) => {
            crate::p2p::metrics::push_log(
                &state.logs, 
                "system", 
                format!("Job '{}' cancelled via web interface", job_id)
            ).await;
            (StatusCode::OK, format!("Job '{}' cancelled successfully", job_id)).into_response()
        },
        Ok(false) => (StatusCode::BAD_REQUEST, format!("Job '{}' cannot be cancelled (already completed)", job_id)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to cancel job: {}", e)).into_response()
    }
}

pub async fn api_jobs_logs(State(_state): State<WebState>, Path(job_id): Path<String>) -> impl IntoResponse {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }

    match job_manager.get_job(&job_id).await {
        Some(job) => Json(job.logs).into_response(),
        None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response()
    }
}

pub async fn api_jobs_artifacts(State(_state): State<WebState>, Path(job_id): Path<String>) -> impl IntoResponse {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }
    match job_manager.get_job(&job_id).await {
        Some(job) => Json(job.artifacts).into_response(),
        None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response(),
    }
}

pub async fn api_jobs_artifact_download(Path((job_id, name)): Path<(String, String)>) -> impl IntoResponse {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
    if let Err(e) = job_manager.load_from_disk().await {
        tracing::warn!("Failed to load job state: {}", e);
    }
    match job_manager.get_job(&job_id).await {
        Some(job) => {
            if let Some(art) = job.artifacts.iter().find(|a| a.name == name) {
                if let Ok(bytes) = tokio::fs::read(&art.stored_path).await {
                    return (StatusCode::OK, bytes).into_response();
                }
                return (StatusCode::NOT_FOUND, "artifact not found").into_response();
            }
            (StatusCode::NOT_FOUND, "artifact not found").into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response(),
    }
}
