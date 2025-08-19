use crate::job_manager::JobManager;
use crate::p2p::metrics;
use crate::policy::{ExecutionPolicy, load_policy};
use crate::storage::ContentStore;

pub async fn execute_native_job(
    job_mgr: &JobManager,
    job_id: &str,
    job: &common::JobSpec,
    binary: &str,
    sha256_hex: Option<String>,
    args: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
    logs: &metrics::SharedLogs,
    cancel_rx: Option<&mut tokio::sync::oneshot::Receiver<()>>,
    storage: Option<crate::p2p::storage::P2PStorage>,
) -> Result<String, String> {
    use crate::p2p::{metrics::push_log, handlers};
    use tokio::process::Command;

    let policy: ExecutionPolicy = load_policy();
    if !policy.allow_native_execution {
        let msg = format!(
            "job failed: native execution is disabled by policy for job {}. {}",
            job.name,
            crate::policy::policy_enable_help()
        );
        return Err(msg);
    }

    let label = format!("job:{}", job.name);
    push_log(logs, &label, format!("staging native binary from {binary}")).await;
    let _ = job_mgr.add_job_log(job_id, "info".to_string(), format!("Staging native from {}", binary)).await;

    let bytes = if let Some(hex) = &sha256_hex {
        if binary.starts_with("cached:") || binary.starts_with("cas:") {
            let store = ContentStore::open();
            if let Some(path) = store.get_path(hex) {
                tokio::fs::read(path).await.map_err(|e| format!("cas read failed: {e}"))?
            } else if let Some(sto) = &storage {
                if let Some(bytes) = sto.get(hex.clone(), std::time::Duration::from_secs(5)).await {
                    let _ = store.put_bytes(&bytes);
                    bytes
                } else {
                    return Err("job failed: digest not available via P2P".to_string());
                }
            } else { return Err("job failed: digest not local and no P2P storage available".to_string()); }
        } else {
            match handlers::fetch_bytes(binary).await {
                Ok(b) => b,
                Err(_) => {
                    if let Some(sto) = &storage {
                        if let Some(bytes) = sto.get(hex.clone(), std::time::Duration::from_secs(5)).await { bytes } else { return Err("job failed: fetch unavailable and P2P fetch failed".to_string()); }
                    } else { return Err("job failed: unsupported source and no P2P storage".to_string()); }
                }
            }
        }
    } else {
        match handlers::fetch_bytes(binary).await { Ok(b) => b, Err(e) => { return Err(format!("job failed: fetch: {e}")); } }
    };

    if let Some(hex) = sha256_hex {
        let d = common::sha256_hex(&bytes);
        if d != hex { return Err("job failed: digest mismatch".to_string()); }
    }

    // Store in CAS
    let store = ContentStore::open();
    let digest = store.put_bytes(&bytes).map_err(|e| format!("cas put failed: {e}"))?;
    let file_path = store.get_path(&digest).ok_or_else(|| "cas path missing".to_string())?;
    // Ensure executable bit on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = tokio::fs::metadata(&file_path).await { let mut p = meta.permissions(); p.set_mode(0o755); let _ = tokio::fs::set_permissions(&file_path, p).await; }
    }

    push_log(logs, &label, format!("starting native job: {} {}", file_path.display(), args.join(" "))).await;
    let _ = job_mgr.add_job_log(job_id, "info".to_string(), "Executing native binary".to_string()).await;

    let mut cmd = Command::new(&file_path);
    cmd.args(&args);
    // set environment
    for (k, v) in env.into_iter() { cmd.env(k, v); }
    // working dir if provided
    if let Some(dir) = &job.execution.working_dir { cmd.current_dir(dir); }

    // Pre-stage attachments if requested
    for item in job.execution.pre_stage.iter() {
        // Expect source in form cas:<sha256>
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
    // spawn child and handle cancellation
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let wait_fut = child.wait();
    let status = if let Some(cancel_rx) = cancel_rx {
        tokio::select! {
            res = wait_fut => res.map_err(|e| e.to_string())?,
            _ = cancel_rx => {
                let _ = child.start_kill();
                return Err("Job cancelled".to_string());
            }
        }
    } else {
        wait_fut.await.map_err(|e| e.to_string())?
    };

    if status.success() {
        // Capture artifacts if requested
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
        Ok(format!("job ok: {}", job.name))
    } else {
        Err(format!("job error: {}: native exit code {:?}", job.name, status.code()))
    }
}


