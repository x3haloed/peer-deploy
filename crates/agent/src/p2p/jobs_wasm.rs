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
) -> Result<String, String> {
    use crate::p2p::{metrics::push_log, state};

    let label = format!("job:{}", job.name);
    push_log(logs, &label, format!("staging wasm from {source}")).await;
    let _ = job_mgr.add_job_log(job_id, "info".to_string(), format!("Staging WASM from {}", source)).await;

    let bytes = match handlers::fetch_bytes(source).await {
        Ok(b) => b,
        Err(e) => {
            let error_msg = format!("job failed: fetch: {e}");
            return Err(error_msg);
        }
    };

    if let Some(hex) = sha256_hex {
        let d = common::sha256_hex(&bytes);
        if d != hex { return Err("job failed: digest mismatch".to_string()); }
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


