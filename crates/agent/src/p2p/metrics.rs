use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::Duration;

use tracing::{info, warn};

/// Shared log buffer used by the HTTP metrics server.
pub type SharedLogs = Arc<tokio::sync::Mutex<std::collections::BTreeMap<String, std::collections::VecDeque<String>>>>;

const LOGS_CAP: usize = 1000;

/// Push a line into the log buffer for a given component, trimming when full.
pub async fn push_log(logs: &SharedLogs, name: &str, line: impl Into<String>) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut map = logs.lock().await;
    let buf = map
        .entry(name.to_string())
        .or_insert_with(|| std::collections::VecDeque::with_capacity(LOGS_CAP));
    if buf.len() >= LOGS_CAP {
        buf.pop_front();
    }
    buf.push_back(format!("{} | {}", now, line.into()));
}

/// Simple in-memory metrics exposed via a tiny HTTP server in Prometheus format.
pub struct Metrics {
    pub status_published_total: AtomicU64,
    pub status_publish_errors_total: AtomicU64,
    pub commands_received_total: AtomicU64,
    pub run_ok_total: AtomicU64,
    pub run_error_total: AtomicU64,
    pub manifest_accepted_total: AtomicU64,
    pub manifest_rejected_total: AtomicU64,
    pub upgrade_accepted_total: AtomicU64,
    pub upgrade_rejected_total: AtomicU64,
    pub agent_version: AtomicU64,
    pub manifest_version: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            status_published_total: AtomicU64::new(0),
            status_publish_errors_total: AtomicU64::new(0),
            commands_received_total: AtomicU64::new(0),
            run_ok_total: AtomicU64::new(0),
            run_error_total: AtomicU64::new(0),
            manifest_accepted_total: AtomicU64::new(0),
            manifest_rejected_total: AtomicU64::new(0),
            upgrade_accepted_total: AtomicU64::new(0),
            upgrade_rejected_total: AtomicU64::new(0),
            agent_version: AtomicU64::new(0),
            manifest_version: AtomicU64::new(0),
        }
    }

    pub fn set_agent_version(&self, v: u64) {
        self.agent_version.store(v, Ordering::Relaxed);
    }
    pub fn set_manifest_version(&self, v: u64) {
        self.manifest_version.store(v, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE agent_status_published_total counter\n");
        out.push_str(&format!(
            "agent_status_published_total {}\n",
            self.status_published_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_status_publish_errors_total counter\n");
        out.push_str(&format!(
            "agent_status_publish_errors_total {}\n",
            self.status_publish_errors_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_commands_received_total counter\n");
        out.push_str(&format!(
            "agent_commands_received_total {}\n",
            self.commands_received_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_run_ok_total counter\n");
        out.push_str(&format!(
            "agent_run_ok_total {}\n",
            self.run_ok_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_run_error_total counter\n");
        out.push_str(&format!(
            "agent_run_error_total {}\n",
            self.run_error_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_manifest_accepted_total counter\n");
        out.push_str(&format!(
            "agent_manifest_accepted_total {}\n",
            self.manifest_accepted_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_manifest_rejected_total counter\n");
        out.push_str(&format!(
            "agent_manifest_rejected_total {}\n",
            self.manifest_rejected_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_upgrade_accepted_total counter\n");
        out.push_str(&format!(
            "agent_upgrade_accepted_total {}\n",
            self.upgrade_accepted_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_upgrade_rejected_total counter\n");
        out.push_str(&format!(
            "agent_upgrade_rejected_total {}\n",
            self.upgrade_rejected_total.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE agent_version gauge\n");
        out.push_str(&format!(
            "agent_version {}\n",
            self.agent_version.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE manifest_version gauge\n");
        out.push_str(&format!(
            "manifest_version {}\n",
            self.manifest_version.load(Ordering::Relaxed)
        ));
        out
    }
}

/// Spawn a minimal HTTP server serving metrics and logs.
pub async fn serve_metrics(metrics: Arc<Metrics>, logs: SharedLogs, bind_addr: &str) {
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!(address=%bind_addr, error=%e, "metrics server bind failed");
            return;
        }
    };
    info!(address=%bind_addr, "metrics server listening");
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                let m = metrics.clone();
                let logs = logs.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 2048];
                    let _ = tokio::time::timeout(
                        Duration::from_millis(500),
                        tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                    )
                    .await;
                    let req = String::from_utf8_lossy(&buf);
                    let mut path = "/metrics";
                    if let Some(line) = req.lines().next() {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            path = parts[1];
                        }
                    }

                    let (status_line, content_type, body) = if path.starts_with("/metrics") {
                        (
                            "HTTP/1.1 200 OK",
                            "text/plain; version=0.0.4",
                            m.render_prometheus(),
                        )
                    } else if path.starts_with("/logs") {
                        // parse query: /logs?component=name&tail=100
                        let mut component: Option<String> = None;
                        let mut tail: usize = 100;
                        if let Some(q) = path.split('?').nth(1) {
                            for pair in q.split('&') {
                                let mut it = pair.split('=');
                                if let (Some(k), Some(v)) = (it.next(), it.next()) {
                                    if k == "component" && !v.is_empty() {
                                        component = Some(v.to_string());
                                    }
                                    if k == "tail" {
                                        if let Ok(n) = v.parse::<usize>() {
                                            tail = n.min(1000);
                                        }
                                    }
                                }
                            }
                        }
                        let mut out = String::new();
                        let map = logs.lock().await;
                        if let Some(name) = component {
                            if let Some(buf) = map.get(&name) {
                                let start = if buf.len() > tail { buf.len() - tail } else { 0 };
                                for line in buf.iter().skip(start) {
                                    out.push_str(line);
                                    out.push('\n');
                                }
                            } else {
                                out.push_str("unknown component\n");
                            }
                        } else {
                            out.push_str("components:\n");
                            for k in map.keys() {
                                out.push_str(k);
                                out.push('\n');
                            }
                        }
                        (
                            "HTTP/1.1 200 OK",
                            "text/plain; charset=utf-8",
                            out,
                        )
                    } else {
                        (
                            "HTTP/1.1 404 Not Found",
                            "text/plain; charset=utf-8",
                            "not found".to_string(),
                        )
                    };

                    let resp = format!(
                        "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, resp.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
                });
            }
            Err(e) => {
                warn!(error=%e, "metrics accept error");
            }
        }
    }
}

