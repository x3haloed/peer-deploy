use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use super::types::*;
use common::Manifest;

pub async fn api_component_restart(State(state): State<WebState>) -> impl IntoResponse {
    state.metrics.inc_restarts_total();
    crate::p2p::metrics::push_log(&state.logs, "system", "Component restart triggered via web interface".to_string()).await;
    (StatusCode::OK, "Component restart triggered").into_response()
}

pub async fn api_component_stop(State(state): State<WebState>, Path(component_name): Path<String>) -> impl IntoResponse {
    if component_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Component name cannot be empty").into_response();
    }
    let desired_components = state.supervisor.get_desired_snapshot().await;
    if !desired_components.contains_key(&component_name) {
        return (StatusCode::NOT_FOUND, format!("Component '{}' not found", component_name)).into_response();
    }
    match stop_component(&state, &component_name).await {
        Ok(_) => {
            crate::p2p::metrics::push_log(&state.logs, "system", format!("Component '{}' stopped via web interface", component_name)).await;
            (StatusCode::OK, format!("Component '{}' stopped successfully", component_name)).into_response()
        }
        Err(e) => {
            crate::p2p::metrics::push_log(&state.logs, "system", format!("Failed to stop component '{}': {}", component_name, e)).await;
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to stop component: {}", e)).into_response()
        }
    }
}

async fn stop_component(state: &WebState, component_name: &str) -> Result<(), String> {
    state.supervisor.cleanup_component(component_name).await;
    remove_component_from_persistent_manifest(component_name)?;
    state.metrics.dec_components_running();
    Ok(())
}

fn remove_component_from_persistent_manifest(component_name: &str) -> Result<(), String> {
    use crate::p2p::state::{load_desired_manifest, save_desired_manifest};
    let manifest_toml = load_desired_manifest().ok_or_else(|| "No persistent manifest found".to_string())?;
    let mut manifest: Manifest = toml::from_str(&manifest_toml).map_err(|e| format!("Failed to parse manifest: {}", e))?;
    if manifest.components.remove(component_name).is_none() {
        return Err(format!("Component '{}' not found in manifest", component_name));
    }
    let updated_toml = toml::to_string(&manifest).map_err(|e| format!("Failed to serialize manifest: {}", e))?;
    save_desired_manifest(&updated_toml);
    Ok(())
}


