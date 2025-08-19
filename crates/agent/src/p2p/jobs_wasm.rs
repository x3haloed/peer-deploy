use crate::job_manager::JobManager;
use crate::p2p::{metrics, handlers};
use crate::storage::ContentStore;

pub async fn execute_wasm_job(
    job_mgr: &JobManager,
    job_id: &str,
    job: &common::JobSpec,
    source: &str,
    sha256_hex: Option<String>,
    memory_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    mounts: Option<Vec<common::MountSpec>>,
    logs: &metrics::SharedLogs,
    cancel_rx: Option<&mut tokio::sync::oneshot::Receiver<()>>,
    storage: Option<crate::p2p::storage::P2PStorage>,
) -> Result<String, String> {
    use crate::p2p::metrics::push_log;

    let label = format!("job:{}", job.name);
    push_log(logs, &label, format!("staging wasm from {source}")).await;
    let _ = job_mgr.add_job_log(job_id, "info".to_string(), format!("Staging WASM from {}", source)).await;

    let bytes = if let Some(hex) = &sha256_hex {
        // Try CAS/P2P first if source isn't directly fetchable or to avoid external transfer
        if source.starts_with("cached:") || source.starts_with("cas:") {
            let digest = hex.clone();
            let store = ContentStore::open();
            if let Some(path) = store.get_path(&digest) {
                tokio::fs::read(path).await.map_err(|e| format!("cas read failed: {e}"))?
            } else if let Some(sto) = &storage {
                if let Some(bytes) = sto.get(digest.clone(), std::time::Duration::from_secs(5)).await {
                    // Save into CAS
                    let _ = store.put_bytes(&bytes);
                    bytes
                } else {
                    return Err("job failed: digest not available via P2P".to_string());
                }
            } else {
                return Err("job failed: digest not local and no P2P storage available".to_string());
            }
        } else {
            // Prefer direct fetch; fallback to P2P if http/file unsupported
            match handlers::fetch_bytes(source).await {
                Ok(b) => b,
                Err(_) => {
                    if let Some(sto) = &storage {
                        if let Some(bytes) = sto.get(hex.clone(), std::time::Duration::from_secs(5)).await {
                            bytes
                        } else {
                            return Err("job failed: fetch unavailable and P2P fetch failed".to_string());
                        }
                    } else {
                        return Err("job failed: unsupported source and no P2P storage".to_string());
                    }
                }
            }
        }
    } else {
        // No digest provided; direct fetch only
        match handlers::fetch_bytes(source).await { Ok(b) => b, Err(e) => { return Err(format!("job failed: fetch: {e}")); } }
    };

    if let Some(hex) = sha256_hex {
        let d = common::sha256_hex(&bytes);
        if d != hex { return Err("job failed: digest mismatch".to_string()); }
    }

    // Pre-stage attachments if requested (write blobs to host before execution)
    for item in job.execution.pre_stage.iter() {
        if let Some(hex) = item.source.strip_prefix("cas:") {
            let store = ContentStore::open();
            let bytes = if let Some(path) = store.get_path(hex) {
                tokio::fs::read(path).await.map_err(|e| format!("prestage cas read failed: {e}"))?
            } else if let Some(sto) = &storage {
                if let Some(bytes) = sto.get(hex.to_string(), std::time::Duration::from_secs(5)).await { bytes } else { return Err(format!("prestage: digest not available via P2P: {hex}")); }
            } else { return Err("prestage: digest not local and no P2P storage available".to_string()); };
            if let Some(parent) = std::path::Path::new(&item.dest).parent() { let _ = tokio::fs::create_dir_all(parent).await; }
            tokio::fs::write(&item.dest, &bytes).await.map_err(|e| format!("prestage write failed: {e}"))?;
        }
    }

    // Store in CAS and execute from there
    let store = ContentStore::open();
    let digest = store.put_bytes(&bytes).map_err(|e| format!("cas put failed: {e}"))?;
    let file_path = store.get_path(&digest).ok_or_else(|| "cas path missing".to_string())?;

    push_log(logs, &label, format!("starting wasm job (mem={}MB fuel={} epoch_ms={})", memory_mb, fuel, epoch_ms)).await;
    let _ = job_mgr.add_job_log(job_id, "info".to_string(), format!("Executing WASM with {}MB memory, {} fuel, {}ms epoch", memory_mb, fuel, epoch_ms)).await;

    let result = if let Some(cancel_rx) = cancel_rx {
        let file_path_str = file_path.display().to_string();
        tokio::select! {
            res = crate::runner::run_wasm_module_with_limits(
                &file_path_str, &label, logs.clone(), memory_mb, fuel, epoch_ms, None, mounts,
            ) => res,
            _ = cancel_rx => {
                let _ = job_mgr.add_job_log(job_id, "warn".to_string(), "Service job cancelled during execution".to_string()).await;
                return Err("Service job cancelled".to_string());
            }
        }
    } else {
        let file_path_str = file_path.display().to_string();
        crate::runner::run_wasm_module_with_limits(
            &file_path_str, &label, logs.clone(), memory_mb, fuel, epoch_ms, None, mounts,
        ).await
    };

    match result {
        Ok(_) => {
            let success_msg = format!("job ok: {}", job.name);
            if let Some(exec) = &job.execution.artifacts {
                for art in exec.iter() {
                    let stored = art.path.clone();
                    let meta = tokio::fs::metadata(&stored).await.ok();
                    let size = meta.as_ref().and_then(|m| if m.is_file() { Some(m.len()) } else { None });
                    let artifact = common::JobArtifact { name: art.name.clone().unwrap_or_else(|| stored.clone()), stored_path: stored, size_bytes: size };
                    let _ = job_mgr.add_job_artifact(job_id, artifact).await;
                }
                if let Some(job_instance) = job_mgr.get_job(job_id).await {
                    let _ = job_mgr.stage_artifacts(job_id, &job_instance.artifacts).await;
                }
            }
            Ok(success_msg)
        }
        Err(e) => Err(format!("job error: {}: {}", job.name, e)),
    }
}


