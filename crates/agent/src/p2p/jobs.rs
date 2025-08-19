use crate::job_manager::JobManager;
use crate::p2p::metrics;

pub async fn execute_oneshot_job(
    job_mgr: std::sync::Arc<JobManager>,
    job_id: String,
    job: common::JobSpec,
    logs: metrics::SharedLogs,
    tx: tokio::sync::mpsc::UnboundedSender<Result<String, String>>,
) {
    let job_runtime = job.runtime.clone();
    match job_runtime {
        common::JobRuntime::Wasm { source, sha256_hex, memory_mb, fuel, epoch_ms, mounts } => {
            let result = super::jobs_wasm::execute_wasm_job(&job_mgr, &job_id, &job, &source, sha256_hex, memory_mb, fuel, epoch_ms, mounts, &logs, None).await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = tx.send(Ok(success_msg));
                },
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = tx.send(Err(error_msg));
                }
            }
        }
    }
}

pub async fn execute_service_job(
    job_mgr: std::sync::Arc<JobManager>,
    job_id: String,
    job: common::JobSpec,
    logs: metrics::SharedLogs,
    tx: tokio::sync::mpsc::UnboundedSender<Result<String, String>>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let job_runtime = job.runtime.clone();
    match job_runtime {
        common::JobRuntime::Wasm { source, sha256_hex, memory_mb, fuel, epoch_ms, mounts } => {
            let result = super::jobs_wasm::execute_wasm_job(&job_mgr, &job_id, &job, &source, sha256_hex, memory_mb, fuel, epoch_ms, mounts, &logs, Some(&mut cancel_rx)).await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr.add_job_log(&job_id, "info".to_string(), "Service job completed normally".to_string()).await;
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = tx.send(Ok(success_msg));
                },
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = tx.send(Err(error_msg));
                }
            }
            job_mgr.unregister_running_job(&job_id).await;
        }
    }
}


