use crate::job_manager::JobManager;
use crate::p2p::metrics;
use crate::p2p::metrics::push_log;
use common::Command;
use uuid::Uuid;

pub async fn execute_oneshot_job_with_broadcast(
    job_mgr: std::sync::Arc<JobManager>,
    job_id: String,
    job: common::JobSpec,
    logs: metrics::SharedLogs,
    tx: tokio::sync::mpsc::UnboundedSender<Result<String, String>>,
    storage: Option<crate::p2p::storage::P2PStorage>,
    broadcast_tx: tokio::sync::mpsc::UnboundedSender<Command>,
    node_id: String,
) {
    let job_runtime = job.runtime.clone();
    let label = format!("job:{}", job.name);
    match job_runtime {
        common::JobRuntime::Wasm {
            source,
            sha256_hex,
            memory_mb,
            fuel,
            epoch_ms,
            mounts,
        } => {
            let result = super::jobs_wasm::execute_wasm_job(
                &job_mgr,
                &job_id,
                &job,
                &source,
                sha256_hex,
                memory_mb,
                fuel,
                epoch_ms,
                mounts,
                &logs,
                None,
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = broadcast_tx.send(Command::JobCompleted {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        exit_code: 0,
                        message_id: Uuid::new_v4().to_string(),
                    });
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = broadcast_tx.send(Command::JobFailed {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        error: error_msg.clone(),
                        message_id: Uuid::new_v4().to_string(),
                    });
                    let _ = tx.send(Err(error_msg));
                }
            }
        }
        common::JobRuntime::Native {
            binary,
            sha256_hex,
            args,
            env,
        } => {
            push_log(&logs, &label, "dispatch: native".to_string()).await;
            let result = super::jobs_native::execute_native_job(
                &job_mgr,
                &job_id,
                &job,
                &binary,
                sha256_hex,
                args,
                env,
                &logs,
                None,
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = broadcast_tx.send(Command::JobCompleted {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        exit_code: 0,
                        message_id: Uuid::new_v4().to_string(),
                    });
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = broadcast_tx.send(Command::JobFailed {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        error: error_msg.clone(),
                        message_id: Uuid::new_v4().to_string(),
                    });
                    let _ = tx.send(Err(error_msg));
                }
            }
        }
        common::JobRuntime::Qemu {
            binary,
            sha256_hex,
            args,
            env,
            target_platform,
            qemu_binary,
        } => {
            push_log(&logs, &label, "dispatch: qemu".to_string()).await;
            let result = super::jobs_qemu::execute_qemu_job(
                &job_mgr,
                &job_id,
                &job,
                &binary,
                sha256_hex,
                args,
                env,
                target_platform,
                qemu_binary,
                &logs,
                None,
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = broadcast_tx.send(Command::JobCompleted {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        exit_code: 0,
                        message_id: Uuid::new_v4().to_string(),
                    });
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = broadcast_tx.send(Command::JobFailed {
                        job_id: job_id.clone(),
                        assigned_node: node_id.clone(),
                        error: error_msg.clone(),
                        message_id: Uuid::new_v4().to_string(),
                    });
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
    storage: Option<crate::p2p::storage::P2PStorage>,
) {
    let job_runtime = job.runtime.clone();
    let label = format!("job:{}", job.name);
    match job_runtime {
        common::JobRuntime::Wasm {
            source,
            sha256_hex,
            memory_mb,
            fuel,
            epoch_ms,
            mounts,
        } => {
            let result = super::jobs_wasm::execute_wasm_job(
                &job_mgr,
                &job_id,
                &job,
                &source,
                sha256_hex,
                memory_mb,
                fuel,
                epoch_ms,
                mounts,
                &logs,
                Some(&mut cancel_rx),
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr
                        .add_job_log(
                            &job_id,
                            "info".to_string(),
                            "Service job completed normally".to_string(),
                        )
                        .await;
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = tx.send(Err(error_msg));
                }
            }
            job_mgr.unregister_running_job(&job_id).await;
        }
        common::JobRuntime::Native {
            binary,
            sha256_hex,
            args,
            env,
        } => {
            push_log(&logs, &label, "dispatch: native".to_string()).await;
            let result = super::jobs_native::execute_native_job(
                &job_mgr,
                &job_id,
                &job,
                &binary,
                sha256_hex,
                args,
                env,
                &logs,
                Some(&mut cancel_rx),
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr
                        .add_job_log(
                            &job_id,
                            "info".to_string(),
                            "Service job completed normally".to_string(),
                        )
                        .await;
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = tx.send(Err(error_msg));
                }
            }
            job_mgr.unregister_running_job(&job_id).await;
        }
        common::JobRuntime::Qemu {
            binary,
            sha256_hex,
            args,
            env,
            target_platform,
            qemu_binary,
        } => {
            push_log(&logs, &label, "dispatch: qemu".to_string()).await;
            let result = super::jobs_qemu::execute_qemu_job(
                &job_mgr,
                &job_id,
                &job,
                &binary,
                sha256_hex,
                args,
                env,
                target_platform,
                qemu_binary,
                &logs,
                Some(&mut cancel_rx),
                storage.clone(),
            )
            .await;
            match result {
                Ok(success_msg) => {
                    let _ = job_mgr
                        .add_job_log(
                            &job_id,
                            "info".to_string(),
                            "Service job completed normally".to_string(),
                        )
                        .await;
                    let _ = job_mgr.complete_job(&job_id, 0).await;
                    let _ = tx.send(Ok(success_msg));
                }
                Err(error_msg) => {
                    let _ = job_mgr.fail_job(&job_id, error_msg.clone()).await;
                    let _ = tx.send(Err(error_msg));
                }
            }
            job_mgr.unregister_running_job(&job_id).await;
        }
    }
}
