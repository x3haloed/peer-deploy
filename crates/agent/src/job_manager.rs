use anyhow::Result;
use common::{JobSpec, JobInstance, JobStatus, JobArtifact};
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

    pub async fn add_job_artifact(&self, job_id: &str, artifact: JobArtifact) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.artifacts.push(artifact);
        }
        drop(state);
        if let Err(e) = self.save_to_disk().await {
            warn!("Failed to save job state after artifact add: {}", e);
        }
        Ok(())
    }

    /// For recurring jobs, decide if they are due and create a new pending instance
    pub async fn evaluate_schedules(&self) -> Result<Vec<JobSpec>> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut due: Vec<JobSpec> = Vec::new();

        let mut state = self.state.lock().await;
        for (_id, job) in state.jobs.iter_mut() {
            if matches!(&job.spec.job_type, common::JobType::Recurring) {
                if let Some(expr) = &job.spec.schedule {
                    // Very small scheduler: support patterns like "*/N * * * *" or "0 H * * *"
                    let is_due = is_cron_due(expr, now, job.last_scheduled_at);
                    if is_due {
                        job.last_scheduled_at = Some(now);
                        due.push(job.spec.clone());
                    }
                }
            }
        }
        Ok(due)
    }
}

fn is_cron_due(expr: &str, now: u64, last_run: Option<u64>) -> bool {
    // Extremely simplified: only minute and hour fields are considered
    // Patterns supported:
    //  - "*/N * * * *" every N minutes
    //  - "0 H * * *" at minute 0 of hour H
    fn minutes_since_epoch(sec: u64) -> u64 { sec / 60 }
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() < 5 { return false; }
    let minute = parts[0];
    let hour = parts[1];
    let now_min = minutes_since_epoch(now);
    let last_min = last_run.map(minutes_since_epoch);

    // every N minutes
    if minute.starts_with("*/") && (hour == "*" || hour == "*") {
        if let Ok(n) = minute.trim_start_matches("*/").parse::<u64>() {
            if n == 0 { return false; }
            // fire when changed bucket and divisible by n
            let due_now = now_min % n == 0;
            let not_already_fired = match last_min { Some(prev) => prev != now_min, None => true };
            return due_now && not_already_fired;
        }
    }
    // at specific hour, minute 0
    if minute == "0" && hour != "*" {
        if let Ok(h) = hour.parse::<u64>() {
            let mins_in_day = 24 * 60;
            let day_min = now_min % mins_in_day;
            let target = h * 60;
            if day_min == target {
                let not_already_fired = match last_min { Some(prev) => prev != now_min, None => true };
                return not_already_fired;
            }
        }
    }
    false
}
