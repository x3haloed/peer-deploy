use axum::{
    extract::{Path, Query, State, Multipart},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};

use super::types::*;
use common::{JobInstance, JobSpec, PreStageSpec, Command};
use base64::Engine as _;
use crate::cmd;
use crate::cmd::util::{new_swarm, mdns_warmup, NodeBehaviourEvent};
use futures::StreamExt;

pub async fn api_jobs_list(State(_state): State<WebState>, Query(params): Query<JobQuery>) -> Json<Vec<JobInstance>> {
    let limit = params.limit.unwrap_or(50) as usize;
    match net_query_jobs(params.status.clone(), limit).await {
        Ok(v) => Json(v),
        Err(e) => {
            tracing::warn!(error=%e, "net_query_jobs failed; falling back to local state");
            let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
            let job_manager = crate::job_manager::JobManager::new(data_dir);
            if let Err(e) = job_manager.load_from_disk().await { tracing::warn!("Failed to load job state: {}", e); }
            let jobs = job_manager.list_jobs(params.status.as_deref(), limit).await;
            Json(jobs)
        }
    }
}

pub async fn api_jobs_submit(State(state): State<WebState>, mut multipart: Multipart) -> impl IntoResponse {
    // Expected fields: job_toml (text), zero or more asset (file)
    let mut job_toml_text: Option<String> = None;
    let mut prestage: Vec<PreStageSpec> = Vec::new();

    while let Ok(Some(mut field)) = multipart.next_field().await {
        let fname = field.name().unwrap_or("").to_string();
        match fname.as_str() {
            "job_toml" | "file" => { job_toml_text = field.text().await.ok(); },
            "asset" => {
                let filename = field.file_name().map(|s| s.to_string()).unwrap_or_else(|| "asset.bin".to_string());
                if let Ok(bytes) = field.bytes().await {
                    // Store locally in CAS
                    let store = crate::storage::ContentStore::open();
                    let digest = store.put_bytes(&bytes).unwrap_or_else(|_| common::sha256_hex(&bytes));
                    // Publish StoragePut to peers
                    if let Ok((mut swarm, topic_cmd, _topic_status)) = crate::cmd::util::new_swarm().await {
                        let _ = libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap());
                        crate::cmd::util::mdns_warmup(&mut swarm).await;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let _ = swarm.behaviour_mut().gossipsub.publish(
                            topic_cmd,
                            common::serialize_message(&Command::StoragePut { digest: digest.clone(), bytes_b64: b64 })
                        );
                    }
                    // Add pre-stage mapping to /tmp/assets/<filename>
                    prestage.push(PreStageSpec { source: format!("cas:{}", digest), dest: format!("/tmp/assets/{}", filename) });
                }
            },
            _ => {}
        }
    }
    let job_toml = match job_toml_text {
        Some(toml) if !toml.is_empty() => toml,
        _ => return (StatusCode::BAD_REQUEST, "Missing job TOML content").into_response()
    };
    let mut job_spec: JobSpec = match toml::from_str(&job_toml) {
        Ok(spec) => spec,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid job TOML: {}", e)).into_response()
    };
    // Inject pre_stage entries
    job_spec.execution.pre_stage.extend(prestage.into_iter());

    match cmd::submit_job_from_spec(job_spec).await {
        Ok(_) => {
            crate::p2p::metrics::push_log(&state.logs, "system", "Job submitted via web interface".to_string()).await;
            (StatusCode::OK, "Job submitted successfully").into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to submit job: {}", e)).into_response()
    }
}

pub async fn api_jobs_get(State(_state): State<WebState>, Path(job_id): Path<String>) -> impl IntoResponse {
    match net_query_job_status(job_id.clone()).await {
        Ok(Some(job)) => Json(job).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response(),
        Err(e) => {
            tracing::warn!(error=%e, "net_query_job_status failed; falling back to local state");
            let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
            let job_manager = crate::job_manager::JobManager::new(data_dir);
            if let Err(e) = job_manager.load_from_disk().await { tracing::warn!("Failed to load job state: {}", e); }
            match job_manager.get_job(&job_id).await {
                Some(job) => Json(job).into_response(),
                None => (StatusCode::NOT_FOUND, format!("Job '{}' not found", job_id)).into_response(),
            }
        }
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


// ======= P2P helpers for network-backed job discovery =======

async fn net_query_jobs(status: Option<String>, limit: usize) -> Result<Vec<JobInstance>, String> {
    let (mut swarm, topic_cmd, topic_status) = new_swarm().await.map_err(|e| e.to_string())?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap())
        .map_err(|e| e.to_string())?;
    mdns_warmup(&mut swarm).await;
    let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), common::serialize_message(&Command::QueryJobs { status_filter: status.clone(), limit }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline.into()) => { return Err("timeout".into()); }
            event = swarm.select_next_some() => {
                if let libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) = event {
                    if let libp2p::gossipsub::Event::Message { message, .. } = ev {
                        if message.topic == topic_status.hash() {
                            if let Ok(list) = common::deserialize_message::<Vec<JobInstance>>(&message.data) {
                                return Ok(list);
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn net_query_job_status(job_id: String) -> Result<Option<JobInstance>, String> {
    let (mut swarm, topic_cmd, topic_status) = new_swarm().await.map_err(|e| e.to_string())?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap())
        .map_err(|e| e.to_string())?;
    mdns_warmup(&mut swarm).await;
    let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), common::serialize_message(&Command::QueryJobStatus { job_id: job_id.clone() }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline.into()) => { return Ok(None); }
            event = swarm.select_next_some() => {
                if let libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) = event {
                    if let libp2p::gossipsub::Event::Message { message, .. } = ev {
                        if message.topic == topic_status.hash() {
                            if let Ok(item) = common::deserialize_message::<JobInstance>(&message.data) {
                                return Ok(Some(item));
                            }
                        }
                    }
                }
            }
        }
    }
}

