mod types;
mod handlers;
mod jobs;
mod websocket;
mod utils;
mod overview;
mod deploy;
mod apply;
mod upgrade;
mod connect;
mod install;
mod discover;
mod components_ops;
mod volumes;
mod history;

pub use types::*;
pub use handlers::{api_status, api_nodes, api_components, api_logs, api_push_multipart, api_upgrade_multipart, api_apply_multipart, api_install_cli, api_install_agent, api_connect_peer, api_discover, api_component_restart, api_component_stop, api_get_policy, api_set_policy, api_qemu_status, api_storage_list, api_storage_pin, api_storage_gc};
pub use jobs::{api_jobs_list, api_jobs_submit, api_jobs_get, api_jobs_cancel, api_jobs_logs, api_jobs_artifacts, api_jobs_artifact_download};
pub use websocket::*;
pub use utils::*;
pub use deploy::{api_deploy, api_deploy_multipart, api_deploy_package_multipart, api_deploy_package_inspect, api_log_components, install_package_from_bytes};
pub use volumes::{api_volumes_list, api_volumes_clear};
pub use history::{api_deploy_history};

use anyhow::Result;
use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use include_dir::{include_dir, Dir};
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::info;

// Embed web assets at compile time
static WEB_ASSETS: Dir<'_> = include_dir!("crates/agent/web");

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
        .route("/api/deploy-multipart", post(api_deploy_multipart))
        .route("/api/deploy-package", post(api_deploy_package_multipart))
        .route("/api/deploy-package/inspect", post(api_deploy_package_inspect))
        .route("/api/discover", post(api_discover))
        .route("/api/connect", post(api_connect_peer))
        .route("/api/log-components", get(api_log_components))
        .route("/api/volumes", get(api_volumes_list))
        .route("/api/volumes/clear", post(api_volumes_clear))
        .route("/api/deploy-history", get(api_deploy_history))
        .route("/api/push", post(api_push_multipart))
        .route("/api/upgrade", post(api_upgrade_multipart))
        .route("/api/apply", post(api_apply_multipart))
        .route("/api/install/cli", post(api_install_cli))
        .route("/api/install/agent", post(api_install_agent))
        .route("/api/components/:name/restart", post(api_component_restart))
        .route("/api/components/:name/stop", post(api_component_stop))
        
        // Job management endpoints
        .route("/api/jobs", get(api_jobs_list))
        .route("/api/jobs/submit", post(api_jobs_submit))
        .route("/api/jobs/:job_id", get(api_jobs_get))
        .route("/api/jobs/:job_id/cancel", post(api_jobs_cancel))
        .route("/api/jobs/:job_id/logs", get(api_jobs_logs))
        .route("/api/jobs/:job_id/artifacts", get(api_jobs_artifacts))
        .route("/api/jobs/:job_id/artifacts/:name", get(api_jobs_artifact_download))
        // Policy and runtime controls
        .route("/api/policy", get(api_get_policy).post(api_set_policy))
        .route("/api/qemu/status", get(api_qemu_status))
        // Storage endpoints
        .route("/api/storage", get(api_storage_list))
        .route("/api/storage/pin", post(api_storage_pin))
        .route("/api/storage/gc", post(api_storage_gc))
        
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
    println!("\n🌐 Management Interface Started");
    println!("   URL: http://127.0.0.1:{}", port);
    println!("   Session ID: {}", session_id);
    println!("   Timeout: {} minutes", timeout_duration.as_secs() / 60);
    println!("   Press Ctrl+C to stop\n");
    
    let app = create_app(state, session_id);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    
    // Set up shutdown handler
    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Shutdown signal received");
    };
    
    // Start the server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    
    println!("Management interface stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_real_data_integration() {
        // Test that the web interface correctly integrates with agent state
        let metrics = std::sync::Arc::new(crate::p2p::metrics::Metrics::new());
        let logs: crate::p2p::metrics::SharedLogs = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
        let supervisor = std::sync::Arc::new(crate::supervisor::Supervisor::new(logs.clone(), metrics.clone()));
        
        // Set some test metrics
        metrics.set_components_desired(2);
        metrics.inc_components_running();
        
        let state = WebState::new(metrics.clone(), logs, supervisor);
        
        // Test the status endpoint
        let response = api_status(axum::extract::State(state)).await;
        let status = response.0;
        
        assert_eq!(status.nodes, 1); // Should show at least 1 node (local)
        assert_eq!(status.components, 1); // Should reflect running components
    }
}
