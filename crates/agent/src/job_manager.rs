use anyhow::Result;
use common::{JobSpec, JobInstance, JobStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Serialize, Deserialize};
use tracing::{info, warn};

type JobId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobManagerState {
    pub jobs: HashMap<JobId, JobInstance>,
    pub next_id: u64,
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
}

impl JobManager {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(JobManagerState::default())),
            data_dir,
        }
    }

    pub async fn load_from_disk(&self) -> Result<()> {
        let state_file = self.data_dir.join("jobs.json");
        if state_file.exists() {
            let content = tokio::fs::read_to_string(&state_file).await?;
            let loaded_state: JobManagerState = serde_json::from_str(&content)?;
            *self.state.lock().await = loaded_state;
            info!("Loaded {} jobs from disk", self.state.lock().await.jobs.len());
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

    pub async fn submit_job(&self, spec: JobSpec) -> Result<String> {
        let mut state = self.state.lock().await;
        let job_id = format!("{}-{}", spec.name, state.next_id);
        state.next_id += 1;

        let mut job = JobInstance::new(job_id.clone(), spec);
        job.add_log("info".to_string(), "Job submitted".to_string());
        
        state.jobs.insert(job_id.clone(), job);
        drop(state);

        // Save to disk
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }

        info!("Job {} submitted successfully", job_id);
        Ok(job_id)
    }

    pub async fn start_job(&self, job_id: &str, node_id: String) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.start(node_id);
            job.add_log("info".to_string(), "Job started".to_string());
        }
        drop(state);
        
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state: {}", e);
        }
        Ok(())
    }

    pub async fn complete_job(&self, job_id: &str, exit_code: i32) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.complete(exit_code);
            job.add_log("info".to_string(), format!("Job completed with exit code {}", exit_code));
        }
        drop(state);
        
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
                
                if let Err(e) = self.save_to_disk().await {
                    warn!("Failed to save job state: {}", e);
                }
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
        
        // Don't save to disk for every log entry to avoid performance issues
        // Logs will be saved when job state changes
        Ok(())
    }
}
