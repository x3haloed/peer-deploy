use axum::{
    extract::{ws::WebSocketUpgrade, State},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use tracing::info;

use super::types::{WebState, WebSocketUpdate};

// WebSocket handler for real-time updates
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

pub async fn handle_websocket(socket: axum::extract::ws::WebSocket, state: WebState) {
    use axum::extract::ws::Message;
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

pub async fn get_status_update(state: &WebState) -> WebSocketUpdate {
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
