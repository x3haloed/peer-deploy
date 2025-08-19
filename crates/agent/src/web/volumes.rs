use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use super::types::WebState;

#[derive(Serialize)]
struct VolumeInfo {
    name: String,
    path: String,
    size_mb: u64,
    files: usize,
}

#[derive(Deserialize)]
pub struct ClearReq { pub name: String }

pub async fn api_volumes_list(State(_state): State<WebState>) -> impl IntoResponse {
    let base = crate::p2p::state::agent_data_dir().join("state").join("components");
    let mut out: Vec<VolumeInfo> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&base) {
        for entry in read.flatten() {
            if let Ok(md) = entry.metadata() {
                if md.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path();
                    let (files, bytes) = dir_stats(&path);
                    out.push(VolumeInfo { name, path: path.display().to_string(), size_mb: (bytes / (1024 * 1024)) as u64, files });
                }
            }
        }
    }
    Json(out)
}

pub async fn api_volumes_clear(State(_state): State<WebState>, Json(req): Json<ClearReq>) -> impl IntoResponse {
    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "missing name").into_response();
    }
    let path = crate::p2p::state::agent_data_dir().join("state").join("components").join(&req.name);
    if !path.exists() { return (StatusCode::NOT_FOUND, "not found").into_response(); }
    if let Err(e) = clear_dir(&path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("clear failed: {}", e)).into_response();
    }
    (StatusCode::OK, "ok").into_response()
}

fn dir_stats(path: &std::path::Path) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    if let Ok(read) = std::fs::read_dir(path) {
        for entry in read.flatten() {
            if let Ok(md) = entry.metadata() {
                if md.is_dir() {
                    let (f, b) = dir_stats(&entry.path());
                    files += f; bytes += b;
                } else if md.is_file() {
                    files += 1; bytes += md.len();
                }
            }
        }
    }
    (files, bytes)
}

fn clear_dir(path: &std::path::Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() { std::fs::remove_dir_all(&p)?; } else { std::fs::remove_file(&p)?; }
    }
    Ok(())
}


