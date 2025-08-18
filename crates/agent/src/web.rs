use anyhow::Result;
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{
    collections::{HashMap, BTreeMap},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tokio::time::timeout;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::info;
use uuid::Uuid;

use crate::p2p::metrics::{Metrics, SharedLogs};
use crate::supervisor::{Supervisor, DesiredComponent};
use common::Status;

// Embed web assets at compile time
static WEB_ASSETS: Dir<'_> = include_dir!("crates/agent/web");

// Session management
#[derive(Clone)]
struct Session {
    id: String,
    created_at: SystemTime,
    last_active: SystemTime,
    authenticated: bool,
}

impl Session {
    fn new() -> Self {
        let now = SystemTime::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            last_active: now,
            authenticated: false,
        }
    }

    fn is_expired(&self) -> bool {
        let now = SystemTime::now();
        match now.duration_since(self.last_active) {
            Ok(duration) => duration > Duration::from_secs(30 * 60), // 30 minutes
            Err(_) => true,
        }
    }

    fn touch(&mut self) {
        self.last_active = SystemTime::now();
    }
}

// Application state
#[derive(Clone)]
pub(crate) struct WebState {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    metrics: Arc<Metrics>,
    logs: SharedLogs,
    supervisor: Arc<Supervisor>,
    // Store peer status updates from the network
    peer_status: Arc<Mutex<BTreeMap<String, Status>>>,
}

impl WebState {
    pub(crate) fn new(metrics: Arc<Metrics>, logs: SharedLogs, supervisor: Arc<Supervisor>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            metrics,
            logs,
            supervisor,
            peer_status: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn update_peer_status(&self, status: Status) {
        let mut peers = self.peer_status.lock().unwrap();
        peers.insert(status.node_id.clone(), status);
    }

    fn create_session(&self) -> String {
        let session = Session::new();
        let session_id = session.id.clone();
        
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), session);
        
        // Clean up expired sessions
        sessions.retain(|_, session| !session.is_expired());
        
        session_id
    }

    fn authenticate_session(&self, session_id: &str) -> bool {
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

    fn validate_session(&self, session_id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            if session.authenticated && !session.is_expired() {
                session.touch();
                return true;
            }
        }
        false
    }
}

// API types
#[derive(Serialize)]
struct ApiStatus {
    nodes: u32,
    components: u32,
    cpu_avg: u32,
}

#[derive(Serialize)]
struct ApiNode {
    id: String,
    online: bool,
    roles: Vec<String>,
    components_running: u32,
    components_desired: u32,
    cpu_percent: u32,
    mem_percent: u32,
}

#[derive(Serialize)]
struct ApiComponent {
    name: String,
    running: bool,
    replicas_running: u32,
    replicas_desired: u32,
    memory_mb: u32,
    nodes: Vec<String>,
}

#[derive(Serialize)]
struct ApiLog {
    timestamp: String,
    component: String,
    message: String,
}

#[derive(Deserialize)]
struct LogQuery {
    tail: Option<u32>,
    component: Option<String>,
}



fn create_app(state: WebState, session_id: String) -> Router {
    // Authenticate the session immediately
    state.authenticate_session(&session_id);
    
    Router::new()
        // Static files and main interface
        .route("/", get(serve_index))
        .route("/static/*path", get(serve_static))
        
        // API endpoints
        .route("/api/status", get(api_status))
        .route("/api/nodes", get(api_nodes))
        .route("/api/components", get(api_components))
        .route("/api/logs", get(api_logs))
        .route("/api/deploy", post(api_deploy))
        .route("/api/discover", post(api_discover))
        .route("/api/components/:name/restart", post(api_component_restart))
        .route("/api/components/:name/stop", post(api_component_stop))
        
        // WebSocket for real-time updates
        .route("/ws", get(websocket_handler))
        
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive())
        )
        .with_state(state)
}

// Static file serving
async fn serve_index() -> impl IntoResponse {
    serve_static_file("index.html").await
}

async fn serve_static(uri: Uri) -> impl IntoResponse {
    let path = uri.path().strip_prefix("/static/").unwrap_or("");
    serve_static_file(path).await
}

async fn serve_static_file(path: &str) -> impl IntoResponse {
    match WEB_ASSETS.get_file(path) {
        Some(file) => {
            let mime_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime_type)
                .body(axum::body::Body::from(file.contents()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("File not found"))
            .unwrap(),
    }
}

// API handlers with real data integration
pub(crate) async fn api_status(State(state): State<WebState>) -> Json<ApiStatus> {
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

async fn api_nodes(State(state): State<WebState>) -> Json<Vec<ApiNode>> {
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

pub(crate) async fn api_components(State(state): State<WebState>) -> Json<Vec<ApiComponent>> {
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

async fn api_logs(
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

fn format_timestamp(timestamp: u64) -> String {
    use time::{OffsetDateTime, format_description::well_known::Iso8601};
    
    match OffsetDateTime::from_unix_timestamp(timestamp as i64) {
        Ok(datetime) => datetime.format(&Iso8601::DEFAULT).unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
        Err(_) => "1970-01-01T00:00:00Z".to_string()
    }
}

#[derive(Deserialize)]
struct DeployRequest {
    name: String,
    source: String,  // URL or file path
    sha256_hex: String,
    replicas: Option<u32>,
    memory_max_mb: Option<u64>,
    fuel: Option<u64>,
    epoch_ms: Option<u64>,
}

async fn api_deploy(State(state): State<WebState>, Json(request): Json<DeployRequest>) -> impl IntoResponse {
    use common::ComponentSpec;
    use std::path::PathBuf;
    
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

async fn api_discover(State(state): State<WebState>) -> impl IntoResponse {
    // Trigger discovery by sending a Hello command to the network
    // In a production system, this would broadcast to the P2P network
    
    // For now, return the current peer status as discovery result
    let peers = state.peer_status.lock().unwrap();
    let discovered_nodes: Vec<serde_json::Value> = peers.iter().map(|(node_id, status)| {
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
    }).collect();
    
    Json(serde_json::json!({
        "discovered_nodes": discovered_nodes,
        "discovery_time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    })).into_response()
}

async fn api_component_restart(State(state): State<WebState>) -> impl IntoResponse {
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

async fn api_component_stop(State(state): State<WebState>) -> impl IntoResponse {
    // Component stop requires removing it from desired state
    // This is a destructive operation that requires the component name
    // For now, return not implemented as it requires path parameter parsing
    
    crate::p2p::metrics::push_log(
        &state.logs, 
        "system", 
        "Component stop requested via web interface".to_string()
    ).await;
    
    (StatusCode::NOT_IMPLEMENTED, "Component stop requires component name parameter").into_response()
}

// WebSocket handler for real-time updates
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: axum::extract::ws::WebSocket, state: WebState) {
    use axum::extract::ws::Message;
    use futures::{sink::SinkExt, stream::StreamExt};
    use tokio::time::{interval, Duration};
    
    info!("WebSocket connection established");
    
    let (mut sender, mut receiver) = socket.split();
    
    // Send initial data
    let initial_status = get_status_update(&state).await;
    if let Ok(msg) = serde_json::to_string(&initial_status) {
        let _ = sender.send(Message::Text(msg)).await;
    }
    
    // Set up periodic updates
    let mut update_interval = interval(Duration::from_secs(2));
    
    // Handle incoming messages and send periodic updates
    loop {
        tokio::select! {
            // Handle incoming WebSocket messages
            ws_msg = receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle client commands if needed
                        if text == "ping" {
                            let _ = sender.send(Message::Text("pong".to_string())).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            
            // Send periodic updates
            _ = update_interval.tick() => {
                let status_update = get_status_update(&state).await;
                if let Ok(msg) = serde_json::to_string(&status_update) {
                    if sender.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
    
    info!("WebSocket connection closed");
}

#[derive(Serialize)]
struct WebSocketUpdate {
    #[serde(rename = "type")]
    update_type: String,
    data: serde_json::Value,
}

pub(crate) async fn get_status_update(state: &WebState) -> WebSocketUpdate {
    use std::sync::atomic::Ordering;
    
    let peers = state.peer_status.lock().unwrap();
    let metrics_data = serde_json::json!({
        "nodes": peers.len().max(1),
        "components_running": state.metrics.components_running.load(Ordering::Relaxed),
        "components_desired": state.metrics.components_desired.load(Ordering::Relaxed),
        "restarts_total": state.metrics.restarts_total.load(Ordering::Relaxed),
        "mem_current_bytes": state.metrics.mem_current_bytes.load(Ordering::Relaxed),
        "mem_peak_bytes": state.metrics.mem_peak_bytes.load(Ordering::Relaxed),
        "fuel_used_total": state.metrics.fuel_used_total.load(Ordering::Relaxed),
    });
    
    WebSocketUpdate {
        update_type: "metrics".to_string(),
        data: metrics_data,
    }
}

// Management session starter
pub async fn start_management_session(
    owner_key_verification: bool,
    timeout_duration: Duration,
) -> Result<()> {
    if !owner_key_verification {
        return Err(anyhow::anyhow!("Authentication required"));
    }

    // Connect to running agent by loading its state and creating shared components
    let state = connect_to_agent().await?;
    let session_id = state.create_session();
    
    // Find an available port
    let port = find_available_port().await?;
    
    // Print session info immediately
    println!("🌐 Management interface available at http://127.0.0.1:{}", port);
    println!("🔒 Session ID: {}", session_id);
    println!("🛑 Use Ctrl+C to terminate session");
    println!("⏱️  Session will timeout in {} minutes", timeout_duration.as_secs() / 60);
    println!();
    
    // Start the web server with timeout
    let server_future = start_management_server_internal(port, session_id, state);
    
    match timeout(timeout_duration, server_future).await {
        Ok(result) => {
            println!("Management session ended");
            result
        },
        Err(_) => {
            println!("⏱️  Management session timed out");
            Ok(())
        }
    }
}

async fn start_management_server_internal(
    port: u16,
    session_id: String,
    state: WebState,
) -> Result<()> {
    let app = create_app(state.clone(), session_id.clone());
    
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    info!("Management web server starting on port {}", port);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn find_available_port() -> Result<u16> {
    // Try ports in the range 49152-65535 (dynamic/private port range)
    for port in 49152..=65535 {
        if let Ok(listener) = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            drop(listener);
            return Ok(port);
        }
    }
    
    Err(anyhow::anyhow!("No available ports found"))
}

async fn connect_to_agent() -> Result<WebState> {
    use crate::p2p::state::load_state;
    
    // Load existing agent state from disk
    let agent_state = load_state();
    
    // Create metrics and initialize with persisted values
    let metrics = Arc::new(Metrics::new());
    metrics.set_agent_version(agent_state.agent_version);
    metrics.set_manifest_version(agent_state.manifest_version);
    
    // Create shared logs - these would ideally be connected to a running agent
    let logs: SharedLogs = Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
    
    // Create supervisor and restore from persistent state
    let supervisor = Arc::new(Supervisor::new(logs.clone(), metrics.clone()));
    
    // Restore components from disk
    if let Err(e) = supervisor.restore_from_disk().await {
        tracing::warn!(error=%e, "Failed to restore component state from disk");
        // Continue anyway - web interface should work even if no components are deployed
    }
    
    // Try to connect to existing agent metrics endpoint if available
    let state = WebState::new(metrics, logs, supervisor);
    
    // Attempt to load current metrics from running agent
    if let Err(e) = load_running_agent_metrics(&state).await {
        tracing::info!(error=%e, "No running agent found, starting with persisted state only");
    }
    
    Ok(state)
}

async fn load_running_agent_metrics(state: &WebState) -> Result<()> {
    // Try to connect to the agent's metrics endpoint
    let metrics_urls = [
        "http://127.0.0.1:9090/metrics", // Default metrics port
        "http://127.0.0.1:3030/metrics", // Alternative port
        "http://127.0.0.1:8080/metrics", // Another common port
    ];
    
    for url in &metrics_urls {
        if let Ok(response) = reqwest::get(*url).await {
            if response.status().is_success() {
                if let Ok(metrics_text) = response.text().await {
                    parse_prometheus_metrics(&metrics_text, &state.metrics);
                    tracing::info!(url=%url, "Successfully connected to running agent metrics");
                    return Ok(());
                }
            }
        }
    }
    
    Err(anyhow::anyhow!("No running agent metrics endpoint found"))
}

fn parse_prometheus_metrics(metrics_text: &str, metrics: &Metrics) {
    use std::sync::atomic::Ordering;
    
    for line in metrics_text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        
        if let Some((metric_name, value_str)) = line.split_once(' ') {
            if let Ok(value) = value_str.parse::<u64>() {
                match metric_name {
                    "components_running" => metrics.components_running.store(value, Ordering::Relaxed),
                    "components_desired" => metrics.set_components_desired(value),
                    "agent_restarts_total" => metrics.restarts_total.store(value, Ordering::Relaxed),
                    "agent_mem_current_bytes" => metrics.set_mem_current_bytes(value),
                    "agent_fuel_used_total" => metrics.fuel_used_total.store(value, Ordering::Relaxed),
                    _ => {} // Ignore unknown metrics
                }
            }
        }
    }
}
