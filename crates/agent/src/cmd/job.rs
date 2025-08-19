use anyhow::Context;
use common::{serialize_message, Command, JobSpec, OwnerKeypair, JobInstance};
use std::time::Duration;

use super::util::{new_swarm, mdns_warmup, owner_dir};
use crate::job_manager::JobManager;

pub async fn submit_job(job_toml_path: String) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;

    mdns_warmup(&mut swarm).await;

    // load owner key to ensure presence (no signing yet for job spec; TODO: signed jobs)
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let _kp_bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let _kp: OwnerKeypair = serde_json::from_slice(&_kp_bytes)?;

    let text = tokio::fs::read_to_string(&job_toml_path).await?;
    let spec: JobSpec = toml::from_str(&text)?;

    let msg = Command::SubmitJob(spec.clone());
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&msg))?;
    
    println!("Job '{}' submitted successfully", spec.name);
    Ok(())
}

pub async fn list_jobs(status_filter: Option<String>, limit: usize) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    let jobs = job_manager.list_jobs(status_filter.as_deref(), limit).await;
    
    if jobs.is_empty() {
        println!("No jobs found");
    } else {
        print_job_table(&jobs);
    }
    
    Ok(())
}

pub async fn job_status(job_id: String) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    if let Some(job) = job_manager.get_job(&job_id).await {
        print_job_details(&job);
    } else {
        println!("Job '{}' not found", job_id);
    }
    
    Ok(())
}

pub async fn cancel_job(job_id: String) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    if job_manager.cancel_job(&job_id).await? {
        println!("Job '{}' cancelled successfully", job_id);
        
        // Also send network cancel command
        let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
        libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>()
            .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;
        mdns_warmup(&mut swarm).await;

        let msg = Command::CancelJob { job_id: job_id.clone() };
        swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic_cmd.clone(), serialize_message(&msg))?;
    } else {
        println!("Job '{}' not found or cannot be cancelled (already completed)", job_id);
    }
    
    Ok(())
}

pub async fn job_logs(job_id: String, tail: usize, follow: bool) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    if let Some(job) = job_manager.get_job(&job_id).await {
        if follow {
            println!("Following logs for job '{}' (Ctrl+C to stop)...", job_id);
            print_job_logs(&job, tail);
            
            // For follow mode, poll for updates every 2 seconds
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                if let Ok(()) = job_manager.load_from_disk().await {
                    if let Some(updated_job) = job_manager.get_job(&job_id).await {
                        if updated_job.logs.len() > job.logs.len() {
                            // Print new logs
                            let new_logs = &updated_job.logs[job.logs.len()..];
                            for log_entry in new_logs {
                                println!("{} [{}] {}", 
                                         format_timestamp(log_entry.timestamp),
                                         log_entry.level.to_uppercase(),
                                         log_entry.message);
                            }
                        }
                        
                        // Break if job is completed
                        if matches!(updated_job.status, common::JobStatus::Completed | common::JobStatus::Failed | common::JobStatus::Cancelled) {
                            break;
                        }
                    }
                }
            }
        } else {
            print_job_logs(&job, tail);
        }
    } else {
        println!("Job '{}' not found", job_id);
    }
    
    Ok(())
}

// Helper function for web API
pub async fn submit_job_from_spec(spec: JobSpec) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;

    mdns_warmup(&mut swarm).await;

    // load owner key to ensure presence
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let _kp_bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let _kp: OwnerKeypair = serde_json::from_slice(&_kp_bytes)?;

    let msg = Command::SubmitJob(spec.clone());
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&msg))?;
    
    println!("Job '{}' submitted successfully", spec.name);
    Ok(())
}

// Helper functions for formatting output

fn print_job_table(jobs: &[JobInstance]) {
    println!("{:<15} {:<20} {:<10} {:<10} {:<20}", "ID", "NAME", "STATUS", "NODE", "SUBMITTED");
    println!("{}", "-".repeat(80));
    
    for job in jobs.iter().take(50) { // Limit display
        let status = format!("{:?}", job.status);
        let node = job.assigned_node.as_deref().unwrap_or("-");
        let submitted = format_timestamp(job.submitted_at);
        let id_short = if job.id.len() > 12 { &job.id[..12] } else { &job.id };
        let name_short = if job.spec.name.len() > 18 { &job.spec.name[..18] } else { &job.spec.name };
        
        println!("{:<15} {:<20} {:<10} {:<10} {:<20}", 
                 id_short, name_short, status, node, submitted);
    }
}

fn print_job_details(job: &JobInstance) {
    println!("Job Details:");
    println!("  ID: {}", job.id);
    println!("  Name: {}", job.spec.name);
    println!("  Status: {:?}", job.status);
    println!("  Type: {:?}", job.spec.job_type);
    
    if let Some(schedule) = &job.spec.schedule {
        println!("  Schedule: {}", schedule);
    }
    
    println!("  Submitted: {}", format_timestamp(job.submitted_at));
    
    if let Some(started) = job.started_at {
        println!("  Started: {}", format_timestamp(started));
    }
    
    if let Some(completed) = job.completed_at {
        println!("  Completed: {}", format_timestamp(completed));
    }
    
    if let Some(node) = &job.assigned_node {
        println!("  Node: {}", node);
    }
    
    if let Some(exit_code) = job.exit_code {
        println!("  Exit Code: {}", exit_code);
    }
    
    if let Some(error) = &job.error_message {
        println!("  Error: {}", error);
    }
    
    println!("  Runtime: {:?}", job.spec.runtime);
    
    if let Some(targeting) = &job.spec.targeting {
        if let Some(platform) = &targeting.platform {
            println!("  Platform: {}", platform);
        }
        if !targeting.tags.is_empty() {
            println!("  Tags: {}", targeting.tags.join(", "));
        }
        if !targeting.node_ids.is_empty() {
            println!("  Target Nodes: {}", targeting.node_ids.join(", "));
        }
    }
}

fn print_job_logs(job: &JobInstance, tail: usize) {
    let logs_to_show = if job.logs.len() > tail {
        &job.logs[job.logs.len() - tail..]
    } else {
        &job.logs
    };
    
    for log_entry in logs_to_show {
        println!("{} [{}] {}", 
                 format_timestamp(log_entry.timestamp),
                 log_entry.level.to_uppercase(),
                 log_entry.message);
    }
}

fn format_timestamp(unix_timestamp: u64) -> String {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    
    let datetime = UNIX_EPOCH + Duration::from_secs(unix_timestamp);
    let now = SystemTime::now();
    
    match now.duration_since(datetime) {
        Ok(elapsed) => {
            let secs = elapsed.as_secs();
            if secs < 60 {
                format!("{}s ago", secs)
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        Err(_) => "future".to_string(),
    }
}

pub async fn job_artifacts(job_id: String) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    if let Some(job) = job_manager.get_job(&job_id).await {
        if job.artifacts.is_empty() {
            println!("No artifacts found for job '{}'", job_id);
        } else {
            println!("Artifacts for job '{}':", job_id);
            println!("{:<20} {:<15} {:<30}", "NAME", "SIZE", "STORED PATH");
            println!("{}", "-".repeat(70));
            
            for artifact in &job.artifacts {
                let size_str = if let Some(size) = artifact.size_bytes {
                    format!("{} bytes", size)
                } else {
                    "unknown".to_string()
                };
                println!("{:<20} {:<15} {:<30}", artifact.name, size_str, artifact.stored_path);
            }
        }
    } else {
        println!("Job '{}' not found", job_id);
    }
    
    Ok(())
}

pub async fn job_download(job_id: String, artifact_name: String, output: Option<String>) -> anyhow::Result<()> {
    let data_dir = crate::p2p::state::agent_data_dir().join("jobs");
    let job_manager = JobManager::new(data_dir);
    
    if let Err(e) = job_manager.load_from_disk().await {
        eprintln!("Warning: Failed to load job state: {}", e);
    }

    if let Some(job) = job_manager.get_job(&job_id).await {
        if let Some(_artifact) = job.artifacts.iter().find(|a| a.name == artifact_name) {
            // Get staged artifact path
            let staged_path = job_manager.get_artifact_path(&job_id, &artifact_name);
            
            if !staged_path.exists() {
                eprintln!("Artifact '{}' not found in staged location", artifact_name);
                return Ok(());
            }
            
            let output_path = output.unwrap_or_else(|| artifact_name.clone());
            
            tokio::fs::copy(&staged_path, &output_path).await?;
            println!("Downloaded artifact '{}' to '{}'", artifact_name, output_path);
        } else {
            println!("Artifact '{}' not found for job '{}'", artifact_name, job_id);
        }
    } else {
        println!("Job '{}' not found", job_id);
    }
    
    Ok(())
}
