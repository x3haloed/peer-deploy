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
) -> Result<String, String> {
    use crate::p2p::{metrics::push_log, handlers, state};
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

    let bytes = match handlers::fetch_bytes(binary).await {
        Ok(b) => b,
        Err(e) => { return Err(format!("job failed: fetch: {e}")); }
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


