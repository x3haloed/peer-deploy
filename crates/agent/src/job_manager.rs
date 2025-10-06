use anyhow::Result;
use common::{JobArtifact, JobInstance, JobSpec, JobStatus, JobType};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

type JobId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobManagerState {
    pub jobs: HashMap<JobId, JobInstance>,
    pub next_id: u64,
}

#[derive(Debug)]
pub struct RunningJob {
    pub handle: JoinHandle<()>,
    pub cancel_tx: tokio::sync::oneshot::Sender<()>,
}

impl Default for JobManagerState {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            next_id: 1,
        }
    }
}

pub struct JobManager {
    state: Arc<Mutex<JobManagerState>>,
    data_dir: std::path::PathBuf,
    running_jobs: Arc<Mutex<HashMap<JobId, RunningJob>>>,
    node_id: String,
    last_update: AtomicU64,
}

impl JobManager {
    pub fn new(data_dir: std::path::PathBuf, node_id: String) -> Self {
        Self {
            state: Arc::new(Mutex::new(JobManagerState::default())),
            data_dir,
            running_jobs: Arc::new(Mutex::new(HashMap::new())),
            node_id,
            last_update: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            ),
        }
    }

    fn mark_update(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_update.store(now, Ordering::Relaxed);
    }

    pub fn last_update(&self) -> u64 {
        self.last_update.load(Ordering::Relaxed)
    }

    pub async fn load_from_disk(&self) -> Result<()> {
        let state_file = self.data_dir.join("jobs.json");
        if state_file.exists() {
            let content = tokio::fs::read_to_string(&state_file).await?;
            let mut loaded_state: JobManagerState = serde_json::from_str(&content)?;
            for job in loaded_state.jobs.values_mut() {
                if job.updated_at == 0 {
                    job.updated_at = job.submitted_at;
                }
                if job.origin_node_id.is_empty() {
                    job.origin_node_id = self.node_id.clone();
                }
            }
            *self.state.lock().await = loaded_state;
            info!(
                "Loaded {} jobs from disk",
                self.state.lock().await.jobs.len()
            );
        }
        Ok(())
    }

    pub async fn save_to_disk(&self) -> Result<()> {
        let _ = tokio::fs::create_dir_all(&self.data_dir).await;
        let state_file = self.data_dir.join("jobs.json");
        let state = self.state.lock().await;
        let content = serde_json::to_string_pretty(&*state)?;
        tokio::fs::write(&state_file, content).await?;
        Ok(())
    }

    pub async fn submit_job(
        &self,
        spec: JobSpec,
        origin_node_id: Option<String>,
        job_id: Option<String>,
    ) -> Result<String> {
        let mut state = self.state.lock().await;
        let origin = origin_node_id.unwrap_or_else(|| self.node_id.clone());
        let id = match job_id {
            Some(id) => id,
            None => {
                let id = format!("{}-{}", origin, state.next_id);
                state.next_id += 1;
                id
            }
        };

        let mut job = JobInstance::new(id.clone(), origin.clone(), spec);
        job.add_log("info".to_string(), "Job submitted".to_string());

        state.jobs.insert(id.clone(), job);
        drop(state);

        self.mark_update();
        // Save to disk
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }

        info!("Job {} submitted successfully", id);
        Ok(id)
    }

    pub async fn assign_job(&self, job_id: &str, node_id: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.assigned_node = Some(node_id.to_string());
            job.add_log(
                "info".to_string(),
                format!("Job assigned to node {}", node_id),
            );
            job.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        drop(state);

        self.mark_update();

        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }

    pub async fn update_job_assignment(&self, job_id: &str, node_id: &str) -> Result<()> {
        self.assign_job(job_id, node_id).await
    }

    pub async fn start_job(&self, job_id: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.start(
                job.assigned_node
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            job.add_log("info".to_string(), "Job started".to_string());
        }
        drop(state);

        self.mark_update();

        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }

    pub async fn complete_job(&self, job_id: &str, exit_code: i32) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.complete(exit_code);
            job.add_log(
                "info".to_string(),
                format!("Job completed with exit code {}", exit_code),
            );
        }
        drop(state);

        self.mark_update();

        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }

    pub async fn fail_job(&self, job_id: &str, error: String) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.fail(error.clone());
            job.add_log("error".to_string(), format!("Job failed: {}", error));
        }
        drop(state);

        self.mark_update();

        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<bool> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            if matches!(job.status, JobStatus::Pending | JobStatus::Running) {
                job.cancel();
                job.add_log("warn".to_string(), "Job cancelled".to_string());
                drop(state);

                // Send cancellation signal to running job if it exists
                let mut running_jobs = self.running_jobs.lock().await;
                if let Some(running_job) = running_jobs.remove(job_id) {
                    let _ = running_job.cancel_tx.send(());
                    running_job.handle.abort();
                    info!("Sent cancellation signal to running job: {}", job_id);
                }

                if let Err(e) = self.save_to_disk().await {
                    warn!("Failed to save job state: {}", e);
                }
                self.mark_update();
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn get_job(&self, job_id: &str) -> Option<JobInstance> {
        let state = self.state.lock().await;
        state.jobs.get(job_id).cloned()
    }

    pub async fn list_jobs(&self, status_filter: Option<&str>, limit: usize) -> Vec<JobInstance> {
        let state = self.state.lock().await;
        let mut jobs: Vec<JobInstance> = state.jobs.values().cloned().collect();

        // Filter by status if specified
        if let Some(status_str) = status_filter {
            let filter_status = match status_str.to_lowercase().as_str() {
                "pending" => Some(JobStatus::Pending),
                "running" => Some(JobStatus::Running),
                "completed" => Some(JobStatus::Completed),
                "failed" => Some(JobStatus::Failed),
                "cancelled" => Some(JobStatus::Cancelled),
                _ => None,
            };

            if let Some(status) = filter_status {
                jobs.retain(|job| job.status == status);
            }
        }

        // Sort by submission time (newest first)
        jobs.sort_by(|a, b| b.submitted_at.cmp(&a.submitted_at));

        // Limit results
        jobs.truncate(limit);
        jobs
    }

    pub async fn add_job_log(&self, job_id: &str, level: String, message: String) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.add_log(level, message);
        }
        drop(state);

        self.mark_update();

        // Don't save to disk for every log entry to avoid performance issues
        // Logs will be saved when job state changes
        Ok(())
    }

    pub async fn add_job_artifact(&self, job_id: &str, artifact: JobArtifact) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.artifacts.push(artifact);
            job.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        drop(state);
        self.mark_update();
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state after artifact add: {}", e);
        }
        Ok(())
    }

    /// For recurring jobs, decide if they are due and create a new pending instance
    pub async fn evaluate_schedules(&self) -> Result<Vec<JobSpec>> {
        use chrono::{DateTime, Utc};
        let now = Utc::now();
        let mut due: Vec<JobSpec> = Vec::new();

        let mut state = self.state.lock().await;
        for (_id, job) in state.jobs.iter_mut() {
            if matches!(&job.spec.job_type, JobType::Recurring) {
                if let Some(expr) = &job.spec.schedule {
                    match Schedule::from_str(expr) {
                        Ok(schedule) => {
                            let last_run = job
                                .last_scheduled_at
                                .map(|ts| DateTime::from_timestamp(ts as i64, 0).unwrap_or(now));

                            // Check if the job is due to run based on the cron schedule
                            if let Some(next_run) = schedule.upcoming(Utc).next() {
                                let should_run = if let Some(last) = last_run {
                                    next_run > last && next_run <= now
                                } else {
                                    next_run <= now
                                };

                                if should_run {
                                    job.last_scheduled_at = Some(now.timestamp() as u64);
                                    job.updated_at = now.timestamp() as u64;
                                    due.push(job.spec.clone());
                                    info!(
                                        "Job '{}' is due for execution based on schedule '{}'",
                                        job.spec.name, expr
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Invalid cron expression '{}' for job '{}': {}",
                                expr, job.spec.name, e
                            );
                            continue;
                        }
                    }
                }
            }
        }
        if !due.is_empty() {
            self.mark_update();
        }
        Ok(due)
    }

    /// Add methods for tracking running jobs and artifact management
    pub async fn register_running_job(
        &self,
        job_id: String,
        handle: JoinHandle<()>,
        cancel_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let mut running_jobs = self.running_jobs.lock().await;
        running_jobs.insert(job_id, RunningJob { handle, cancel_tx });
    }

    pub async fn unregister_running_job(&self, job_id: &str) -> Option<RunningJob> {
        let mut running_jobs = self.running_jobs.lock().await;
        running_jobs.remove(job_id)
    }

    /// Copy artifacts to job-specific directory
    pub async fn stage_artifacts(
        &self,
        job_id: &str,
        artifacts: &[common::JobArtifact],
    ) -> Result<()> {
        let artifacts_dir = self
            .data_dir
            .parent()
            .unwrap()
            .join("artifacts")
            .join("jobs")
            .join(job_id);
        tokio::fs::create_dir_all(&artifacts_dir).await?;

        for artifact in artifacts {
            let dest_path = artifacts_dir.join(&artifact.name);
            if let Err(e) = tokio::fs::copy(&artifact.stored_path, &dest_path).await {
                warn!(
                    "Failed to copy artifact '{}' to {}: {}",
                    artifact.name,
                    dest_path.display(),
                    e
                );
            } else {
                info!(
                    "Staged artifact '{}' to {}",
                    artifact.name,
                    dest_path.display()
                );
                // Compute digest for downstream reuse (best-effort)
                if let Ok(bytes) = tokio::fs::read(&artifact.stored_path).await {
                    let digest = common::sha256_hex(&bytes);
                    let mut state = self.state.lock().await;
                    if let Some(job) = state.jobs.get_mut(job_id) {
                        if let Some(a) = job.artifacts.iter_mut().find(|a| a.name == artifact.name)
                        {
                            a.sha256_hex = Some(digest);
                            job.updated_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs();
                        }
                    }
                }
            }
        }
        self.mark_update();
        Ok(())
    }

    /// Get artifact path for serving
    pub fn get_artifact_path(&self, job_id: &str, artifact_name: &str) -> std::path::PathBuf {
        self.data_dir
            .parent()
            .unwrap()
            .join("artifacts")
            .join("jobs")
            .join(job_id)
            .join(artifact_name)
    }

    /// Merge remote job state into local store
    pub async fn sync_job(&self, job: JobInstance) {
        let mut state = self.state.lock().await;
        match state.jobs.get_mut(&job.id) {
            Some(local) => {
                if job.updated_at > local.updated_at {
                    *local = job;
                }
            }
            None => {
                state.jobs.insert(job.id.clone(), job);
            }
        }
    }

    /// Merge a list of remote jobs
    pub async fn sync_jobs(&self, jobs: Vec<JobInstance>) -> Result<()> {
        {
            for job in jobs {
                self.sync_job(job).await;
            }
        }
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }
}
