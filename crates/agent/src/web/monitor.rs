use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use std::time::{SystemTime, UNIX_EPOCH};

use super::types::WebState;

#[derive(Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub component: String,
    pub status: HealthStatus,
    pub message: String,
    pub last_check: u64,
    pub response_time_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

#[derive(Serialize, Deserialize)]
pub struct FleetHealth {
    pub overall_status: HealthStatus,
    pub total_nodes: u32,
    pub healthy_nodes: u32,
    pub warning_nodes: u32,
    pub critical_nodes: u32,
    pub total_components: u32,
    pub healthy_components: u32,
    pub failed_components: u32,
    pub average_response_time: f64,
    pub disk_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub uptime_seconds: u64,
    pub last_incident: Option<String>,
    pub checks: Vec<HealthCheckResult>,
}

#[derive(Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_id: String,
    pub status: HealthStatus,
    pub components_running: u32,
    pub components_desired: u32,
    pub cpu_percent: u32,
    pub memory_percent: u32,
    pub disk_usage_percent: f64,
    pub uptime_seconds: u64,
    pub last_seen: u64,
    pub agent_version: u64,
    pub platform: String,
    pub tags: Vec<String>,
    pub alerts: Vec<AlertInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct AlertInfo {
    pub id: String,
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
    pub timestamp: u64,
    pub acknowledged: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub replicas_running: u32,
    pub replicas_desired: u32,
    pub last_restart: Option<u64>,
    pub restart_count: u32,
    pub error_rate: f64,
    pub response_time_p95: f64,
    pub memory_usage_mb: u64,
    pub cpu_usage_percent: f64,
}

/// Get overall fleet health status
pub async fn api_fleet_health(State(state): State<WebState>) -> Json<FleetHealth> {
    let start_time = SystemTime::now();
    let mut checks = Vec::new();
    
    // Basic connectivity check
    let connectivity_check = HealthCheckResult {
        component: "connectivity".to_string(),
        status: HealthStatus::Healthy,
        message: "Agent responding".to_string(),
        last_check: now_unix(),
        response_time_ms: start_time.elapsed().unwrap_or_default().as_millis() as u64,
    };
    checks.push(connectivity_check);
    
    // Component health check
    let desired_components = state.supervisor.get_desired_snapshot().await;
    let total_components = desired_components.len() as u32;
    let mut healthy_components = 0u32;
    let mut failed_components = 0u32;
    
    for (name, desired) in desired_components.iter() {
        let replicas_desired = desired.spec.replicas.unwrap_or(1);
        let replicas_running = get_running_replicas(&state, name).await;
        
        let (status, message) = if replicas_running >= replicas_desired {
            healthy_components += 1;
            (HealthStatus::Healthy, format!("{}/{} replicas running", replicas_running, replicas_desired))
        } else if replicas_running > 0 {
            (HealthStatus::Warning, format!("{}/{} replicas running", replicas_running, replicas_desired))
        } else {
            failed_components += 1;
            (HealthStatus::Critical, format!("Component not running ({}/{} replicas)", replicas_running, replicas_desired))
        };
        
        checks.push(HealthCheckResult {
            component: format!("component:{}", name),
            status,
            message,
            last_check: now_unix(),
            response_time_ms: 0,
        });
    }
    
    // Storage health check
    let storage_check = check_storage_health().await;
    checks.push(storage_check);
    
    // Peer connectivity health
    let peers = state.peer_status.lock().unwrap();
    let total_nodes = peers.len().max(1) as u32; // At least 1 (local)
    let healthy_nodes = peers.values().filter(|p| p.components_running > 0 || p.components_desired > 0).count() as u32;
    let warning_nodes = 0u32; // TODO: Implement node health criteria
    let critical_nodes = total_nodes.saturating_sub(healthy_nodes + warning_nodes);
    
    // Calculate overall status
    let overall_status = if failed_components > 0 || critical_nodes > 0 {
        HealthStatus::Critical
    } else if total_components > healthy_components || warning_nodes > 0 {
        HealthStatus::Warning
    } else {
        HealthStatus::Healthy
    };
    
    // Get system metrics
    let (disk_usage, memory_usage, uptime) = get_system_metrics();
    
    Json(FleetHealth {
        overall_status,
        total_nodes,
        healthy_nodes,
        warning_nodes,
        critical_nodes,
        total_components,
        healthy_components,
        failed_components,
        average_response_time: calculate_average_response_time(&checks),
        disk_usage_percent: disk_usage,
        memory_usage_percent: memory_usage,
        uptime_seconds: uptime,
        last_incident: None, // TODO: Implement incident tracking
        checks,
    })
}

/// Get health status for all nodes in the fleet
pub async fn api_node_health(State(state): State<WebState>) -> Json<Vec<NodeHealth>> {
    let peers = state.peer_status.lock().unwrap();
    let mut nodes = Vec::new();
    
    // Add peer nodes
    for (node_id, status) in peers.iter() {
        let mut alerts = Vec::new();
        
        // Generate alerts based on node metrics
        if status.cpu_percent > 90 {
            alerts.push(AlertInfo {
                id: format!("cpu-{}", node_id),
                severity: AlertSeverity::Critical,
                title: "High CPU Usage".to_string(),
                message: format!("CPU usage at {}%", status.cpu_percent),
                timestamp: now_unix(),
                acknowledged: false,
            });
        }
        
        if status.mem_percent > 85 {
            alerts.push(AlertInfo {
                id: format!("memory-{}", node_id),
                severity: AlertSeverity::Warning,
                title: "High Memory Usage".to_string(),
                message: format!("Memory usage at {}%", status.mem_percent),
                timestamp: now_unix(),
                acknowledged: false,
            });
        }
        
        if status.components_running < status.components_desired {
            alerts.push(AlertInfo {
                id: format!("components-{}", node_id),
                severity: AlertSeverity::Warning,
                title: "Component Drift".to_string(),
                message: format!("Running {}/{} desired components", status.components_running, status.components_desired),
                timestamp: now_unix(),
                acknowledged: false,
            });
        }
        
        let node_status = if !alerts.iter().any(|a| matches!(a.severity, AlertSeverity::Critical)) {
            if alerts.is_empty() { HealthStatus::Healthy } else { HealthStatus::Warning }
        } else {
            HealthStatus::Critical
        };
        
        nodes.push(NodeHealth {
            node_id: node_id.clone(),
            status: node_status,
            components_running: status.components_running as u32,
            components_desired: status.components_desired as u32,
            cpu_percent: status.cpu_percent as u32,
            memory_percent: status.mem_percent as u32,
            disk_usage_percent: 0.0, // TODO: Get disk usage from peers
            uptime_seconds: 0, // TODO: Track peer uptime
            last_seen: now_unix(),
            agent_version: status.agent_version,
            platform: "unknown".to_string(), // TODO: Add platform to Status
            tags: status.tags.clone(),
            alerts,
        });
    }
    
    // Add local node if no peers
    if nodes.is_empty() {
        use std::sync::atomic::Ordering;
        let (disk_usage, memory_usage, uptime) = get_system_metrics();
        
        let mut alerts = Vec::new();
        if memory_usage > 85.0 {
            alerts.push(AlertInfo {
                id: "local-memory".to_string(),
                severity: AlertSeverity::Warning,
                title: "High Memory Usage".to_string(),
                message: format!("Memory usage at {:.1}%", memory_usage),
                timestamp: now_unix(),
                acknowledged: false,
            });
        }
        
        nodes.push(NodeHealth {
            node_id: "local-node".to_string(),
            status: if alerts.is_empty() { HealthStatus::Healthy } else { HealthStatus::Warning },
            components_running: state.metrics.components_running.load(Ordering::Relaxed) as u32,
            components_desired: state.metrics.components_desired.load(Ordering::Relaxed) as u32,
            cpu_percent: 0, // TODO: Get local CPU usage
            memory_percent: memory_usage as u32,
            disk_usage_percent: disk_usage,
            uptime_seconds: uptime,
            last_seen: now_unix(),
            agent_version: 1, // TODO: Get actual version
            platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            tags: vec!["local".to_string()],
            alerts,
        });
    }
    
    Json(nodes)
}

/// Get detailed health for all components
pub async fn api_component_health(State(state): State<WebState>) -> Json<Vec<ComponentHealth>> {
    let desired_components = state.supervisor.get_desired_snapshot().await;
    let mut components = Vec::new();
    
    for (name, desired) in desired_components.iter() {
        let replicas_desired = desired.spec.replicas.unwrap_or(1);
        let replicas_running = get_running_replicas(&state, name).await;
        
        let status = if replicas_running >= replicas_desired {
            HealthStatus::Healthy
        } else if replicas_running > 0 {
            HealthStatus::Warning
        } else {
            HealthStatus::Critical
        };
        
        // Get component metrics from logs/supervisor
        let (restart_count, error_rate) = get_component_metrics(&state, name).await;
        
        components.push(ComponentHealth {
            name: name.clone(),
            status,
            replicas_running,
            replicas_desired,
            last_restart: None, // TODO: Track restart times
            restart_count,
            error_rate,
            response_time_p95: 0.0, // TODO: Track response times
            memory_usage_mb: desired.spec.memory_max_mb.unwrap_or(64),
            cpu_usage_percent: 0.0, // TODO: Track CPU per component
        });
    }
    
    Json(components)
}

/// Acknowledge an alert
pub async fn api_acknowledge_alert(State(_state): State<WebState>) -> impl IntoResponse {
    // TODO: Implement alert acknowledgment persistence
    (StatusCode::OK, "Alert acknowledged")
}

// Helper functions

async fn get_running_replicas(state: &WebState, component_name: &str) -> u32 {
    // Heuristic: check if component has recent logs
    let logs_map = state.logs.lock().await;
    if let Some(logs) = logs_map.get(component_name) {
        if !logs.is_empty() {
            // If we have logs, assume it's running
            return 1; // TODO: Better replica tracking
        }
    }
    0
}

async fn check_storage_health() -> HealthCheckResult {
    let store = crate::storage::ContentStore::open();
    let entries = store.list();
    let total_size_mb = entries.iter().map(|(_, e)| e.size_bytes).sum::<u64>() / (1024 * 1024);
    
    let (status, message) = if total_size_mb > 10_000 { // 10GB threshold
        (HealthStatus::Warning, format!("Storage usage high: {} MB", total_size_mb))
    } else {
        (HealthStatus::Healthy, format!("Storage usage: {} MB, {} blobs", total_size_mb, entries.len()))
    };
    
    HealthCheckResult {
        component: "storage".to_string(),
        status,
        message,
        last_check: now_unix(),
        response_time_ms: 0,
    }
}

fn get_system_metrics() -> (f64, f64, u64) {
    // Basic system metrics - in production, use proper system monitoring
    let disk_usage = 0.0; // TODO: Implement disk usage check
    let memory_usage = get_memory_usage_percent();
    let uptime = get_uptime_seconds();
    (disk_usage, memory_usage, uptime)
}

fn get_memory_usage_percent() -> f64 {
    // Simple memory usage estimation
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
            let mut total = 0u64;
            let mut available = 0u64;
            
            for line in contents.lines() {
                if line.starts_with("MemTotal:") {
                    total = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                } else if line.starts_with("MemAvailable:") {
                    available = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                }
            }
            
            if total > 0 {
                return ((total - available) as f64 / total as f64) * 100.0;
            } else {
                return 0.0;
            }
        } else {
            return 0.0;
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        // On macOS, use vm_stat or just return a placeholder
        45.0 // Placeholder
    }
    
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0.0
    }
}

fn get_uptime_seconds() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/uptime") {
            if let Some(uptime_str) = contents.split_whitespace().next() {
                if let Ok(uptime) = uptime_str.parse::<f64>() {
                    return uptime as u64;
                }
            }
        }
    }
    
    // Fallback: time since unix epoch (not real uptime)
    now_unix()
}

async fn get_component_metrics(state: &WebState, component_name: &str) -> (u32, f64) {
    // Analyze logs for restart patterns and errors
    let logs_map = state.logs.lock().await;
    if let Some(logs) = logs_map.get(component_name) {
        let restart_count = logs.iter()
            .filter(|log| log.contains("launching replica") || log.contains("replica crashed"))
            .count() as u32;
        
        let error_count = logs.iter()
            .filter(|log| log.contains("error") || log.contains("Error") || log.contains("failed"))
            .count();
        
        let error_rate = if logs.len() > 0 {
            (error_count as f64 / logs.len() as f64) * 100.0
        } else {
            0.0
        };
        
        return (restart_count, error_rate);
    }
    
    (0, 0.0)
}

fn calculate_average_response_time(checks: &[HealthCheckResult]) -> f64 {
    if checks.is_empty() {
        return 0.0;
    }
    
    let total: u64 = checks.iter().map(|c| c.response_time_ms).sum();
    total as f64 / checks.len() as f64
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
