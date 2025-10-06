use anyhow::Result;
use std::sync::Arc;

use super::types::WebState;
use crate::p2p::metrics::{Metrics, SharedLogs};
use crate::supervisor::Supervisor;

pub fn format_timestamp(timestamp: u64) -> String {
    use time::{format_description::well_known::Iso8601, OffsetDateTime};

    match OffsetDateTime::from_unix_timestamp(timestamp as i64) {
        Ok(datetime) => datetime
            .format(&Iso8601::DEFAULT)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
        Err(_) => "1970-01-01T00:00:00Z".to_string(),
    }
}

pub async fn connect_to_agent() -> Result<WebState> {
    use crate::p2p::state::load_state;

    // Load existing agent state from disk
    let agent_state = load_state();

    // Create metrics and initialize with persisted values
    let metrics = Arc::new(Metrics::new());
    metrics.set_agent_version(agent_state.agent_version);
    metrics.set_manifest_version(agent_state.manifest_version);

    // Create shared logs - these would ideally be connected to a running agent
    let logs: SharedLogs = Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));

    // Create supervisor and restore from persistent state
    let supervisor = Arc::new(Supervisor::new(logs.clone(), metrics.clone()));

    // Restore components from disk
    if let Err(e) = supervisor.restore_from_disk().await {
        tracing::warn!(error=%e, "Failed to restore component state from disk");
        // Continue anyway - web interface should work even if no components are deployed
    }

    // Try to connect to existing agent metrics endpoint if available
    let state = WebState::new(metrics, logs, supervisor);

    // Attempt to load current metrics from running agent
    if let Err(e) = load_running_agent_metrics(&state).await {
        tracing::info!(error=%e, "No running agent found, starting with persisted state only");
    }

    Ok(state)
}

async fn load_running_agent_metrics(state: &WebState) -> Result<()> {
    // Try to connect to the agent's metrics endpoint
    let metrics_urls = [
        "http://127.0.0.1:9920/metrics", // Default metrics port in agent
        "http://127.0.0.1:9090/metrics",
        "http://127.0.0.1:3030/metrics",
        "http://127.0.0.1:8080/metrics",
    ];

    for url in &metrics_urls {
        if let Ok(response) = reqwest::get(*url).await {
            if response.status().is_success() {
                if let Ok(metrics_text) = response.text().await {
                    parse_prometheus_metrics(&metrics_text, &state.metrics);
                    tracing::info!(url=%url, "Successfully connected to running agent metrics");
                    return Ok(());
                }
            }
        }
    }

    Err(anyhow::anyhow!("No running agent metrics endpoint found"))
}

fn parse_prometheus_metrics(metrics_text: &str, metrics: &Metrics) {
    use std::sync::atomic::Ordering;

    for line in metrics_text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        if let Some((metric_name, value_str)) = line.split_once(' ') {
            if let Ok(value) = value_str.parse::<u64>() {
                match metric_name {
                    "components_running" => {
                        metrics.components_running.store(value, Ordering::Relaxed)
                    }
                    "components_desired" => metrics.set_components_desired(value),
                    "agent_restarts_total" => {
                        metrics.restarts_total.store(value, Ordering::Relaxed)
                    }
                    "agent_mem_current_bytes" => metrics.set_mem_current_bytes(value),
                    "agent_fuel_used_total" => {
                        metrics.fuel_used_total.store(value, Ordering::Relaxed)
                    }
                    _ => {} // Ignore unknown metrics
                }
            }
        }
    }
}

pub async fn find_available_port() -> Result<u16> {
    // Try ports in the range 49152-65535 (dynamic/private port range)
    for port in 49152..=65535 {
        if let Ok(listener) = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            drop(listener);
            return Ok(port);
        }
    }

    Err(anyhow::anyhow!("No available ports found"))
}
