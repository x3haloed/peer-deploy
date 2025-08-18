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
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tokio::time::timeout;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::info;
use uuid::Uuid;

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
pub struct WebState {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    // We'll add references to agent state here later
}

impl WebState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
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

// Web server implementation
pub async fn start_management_server(
    port: u16,
    session_id: String,
    state: WebState,
) -> Result<()> {
    let app = create_app(state.clone(), session_id.clone());
    
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    info!("🌐 Management interface available at http://127.0.0.1:{}", port);
    info!("🔒 Session ID: {}", session_id);
    info!("🛑 Use Ctrl+C to terminate session");
    
    axum::serve(listener, app).await?;
    
    Ok(())
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

// API handlers (mock implementations for now)
async fn api_status(State(_state): State<WebState>) -> Json<ApiStatus> {
    // TODO: Get real data from agent state
    Json(ApiStatus {
        nodes: 1,
        components: 0,
        cpu_avg: 15,
    })
}

async fn api_nodes(State(_state): State<WebState>) -> Json<Vec<ApiNode>> {
    // TODO: Get real node data
    Json(vec![
        ApiNode {
            id: "local-node-001".to_string(),
            online: true,
            roles: vec!["development".to_string()],
            components_running: 0,
            components_desired: 0,
            cpu_percent: 15,
            mem_percent: 35,
        }
    ])
}

async fn api_components(State(_state): State<WebState>) -> Json<Vec<ApiComponent>> {
    // TODO: Get real component data
    Json(vec![])
}

async fn api_logs(
    State(_state): State<WebState>,
    Query(params): Query<LogQuery>,
) -> Json<Vec<ApiLog>> {
    // TODO: Get real logs from agent
    let tail = params.tail.unwrap_or(100);
    let _component = params.component.unwrap_or_else(|| "__all__".to_string());
    
    Json(vec![
        ApiLog {
            timestamp: "2024-01-01 00:00:00".to_string(),
            component: "system".to_string(),
            message: "Management interface initialized".to_string(),
        }
    ])
}

async fn api_deploy(State(_state): State<WebState>) -> impl IntoResponse {
    // TODO: Implement real deployment
    StatusCode::NOT_IMPLEMENTED
}

async fn api_discover(State(_state): State<WebState>) -> impl IntoResponse {
    // TODO: Implement node discovery
    StatusCode::OK
}

async fn api_component_restart(State(_state): State<WebState>) -> impl IntoResponse {
    // TODO: Implement component restart
    StatusCode::NOT_IMPLEMENTED
}

async fn api_component_stop(State(_state): State<WebState>) -> impl IntoResponse {
    // TODO: Implement component stop
    StatusCode::NOT_IMPLEMENTED
}

// WebSocket handler for real-time updates
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(_state): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(handle_websocket)
}

async fn handle_websocket(socket: axum::extract::ws::WebSocket) {
    // TODO: Implement real-time updates
    info!("WebSocket connection established");
    
    // For now, just accept the connection and do nothing
    let _ = socket;
}

// Management session starter
pub async fn start_management_session(
    owner_key_verification: bool, // TODO: implement proper auth
    timeout_duration: Duration,
) -> Result<()> {
    if !owner_key_verification {
        return Err(anyhow::anyhow!("Authentication required"));
    }

    let state = WebState::new();
    let session_id = state.create_session();
    
    // Find an available port
    let port = find_available_port().await?;
    
    // Start the web server with timeout
    let server_future = start_management_server(port, session_id, state);
    
    match timeout(timeout_duration, server_future).await {
        Ok(result) => result,
        Err(_) => {
            info!("Management session timed out");
            Ok(())
        }
    }
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
