use axum::{
    extract::{Path, Query, State, Multipart},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};

use super::types::*;
use common::{JobInstance, JobSpec};
use crate::cmd;

pub async fn api_jobs_list(State(_state): State<WebState>, Query(params): Query<JobQuery>) -> Json<Vec<JobInstance>> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = crate::job_manager::JobManager::new(data_dir);
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
            "job_toml" | "file" => { job_toml_text = field.text().await.ok(); },
            _ => {}
        }
    }
    let job_toml = match job_toml_text {
        Some(toml) if !toml.is_empty() => toml,
        _ => return (StatusCode::BAD_REQUEST, "Missing job TOML content").into_response()
    };
    let job_spec: JobSpec = match toml::from_str(&job_toml) {
        Ok(spec) => spec,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid job TOML: {}", e)).into_response()
    };
    match cmd::submit_job_from_spec(job_spec).await {
        Ok(_) => {
            crate::p2p::metrics::push_log(&state.logs, "system", "Job submitted via web interface".to_string()).await;
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
            crate::p2p::metrics::push_log(&state.logs, "system", format!("Job '{}' cancelled via web interface", job_id)).await;
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
            if let Some(_art) = job.artifacts.iter().find(|a| a.name == name) {
                let artifact_path = job_manager.get_artifact_path(&job_id, &name);
                if let Ok(bytes) = tokio::fs::read(&artifact_path).await {
                    let content_type = mime_guess::from_path(&artifact_path).first_or_octet_stream().to_string();
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, content_type)
                        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", name))
                        .body(axum::body::Body::from(bytes))
                        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build response").into_response());
                }
                return (StatusCode::NOT_FOUND, "artifact file not found in staged location").into_response();
            }
            (StatusCode::NOT_FOUND, "artifact not found in job metadata").into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response(),
    }
}


