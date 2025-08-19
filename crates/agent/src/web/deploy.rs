use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::path::PathBuf;

use super::types::*;
use crate::supervisor::DesiredComponent;
use common::ComponentSpec;

pub async fn api_deploy(State(state): State<WebState>, Json(request): Json<DeployRequest>) -> impl IntoResponse {
    if request.name.is_empty() || request.source.is_empty() || request.sha256_hex.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing required fields: name, source, sha256_hex").into_response();
    }
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
    let path = if request.source.starts_with("file://") {
        PathBuf::from(request.source.strip_prefix("file://").unwrap_or(&request.source))
    } else {
        return (StatusCode::NOT_IMPLEMENTED, "HTTP sources not yet implemented").into_response();
    };
    if !path.exists() {
        return (StatusCode::BAD_REQUEST, "Component file does not exist").into_response();
    }
    let desired_component = DesiredComponent { name: request.name.clone(), path, spec };
    state.supervisor.upsert_component(desired_component).await;
    crate::p2p::metrics::push_log(&state.logs, "system", format!("Component '{}' deployed via web interface", request.name)).await;
    (StatusCode::OK, "Component deployed successfully").into_response()
}

pub async fn api_deploy_multipart(State(state): State<WebState>, mut multipart: Multipart) -> impl IntoResponse {
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
    let digest = common::sha256_hex(&bin);
    let stage_dir = crate::p2p::state::agent_data_dir().join("artifacts");
    let _ = tokio::fs::create_dir_all(&stage_dir).await;
    let file_path = stage_dir.join(format!("{}-{}.wasm", name, &digest[..16]));
    if !file_path.exists() {
        if tokio::fs::write(&file_path, &bin).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to write artifact").into_response();
        }
    }
    let spec = ComponentSpec { source: format!("cached:{}", digest), sha256_hex: digest.clone(), replicas, memory_max_mb, fuel, epoch_ms, mounts: None, ports: None, visibility: None };
    let desired_component = DesiredComponent { name: name.clone(), path: file_path.clone(), spec: spec.clone() };
    state.supervisor.upsert_component(desired_component).await;
    crate::p2p::state::update_persistent_manifest_with_component(&name, spec);
    let _ = tags_csv;
    crate::p2p::metrics::push_log(&state.logs, "system", format!("Component '{}' deployed via multipart", name)).await;
    (StatusCode::OK, "ok").into_response()
}

pub async fn api_log_components(State(state): State<WebState>) -> Json<Vec<String>> {
    let map = state.logs.lock().await;
    let mut out: Vec<String> = map.keys().cloned().collect();
    out.sort();
    Json(out)
}


