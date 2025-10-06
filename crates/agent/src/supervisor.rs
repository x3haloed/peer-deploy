use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::task::JoinHandle;

use tracing::{info, warn};

use crate::p2p::metrics::{push_log, Metrics, SharedLogs};
use crate::p2p::state::{agent_data_dir, load_desired_manifest};
use crate::runner::run_wasm_module_with_limits;
use common::{sha256_hex, ComponentSpec, Manifest};

#[derive(Clone, Debug)]
pub struct DesiredComponent {
    pub name: String,
    pub path: PathBuf,
    pub spec: ComponentSpec,
}

/// Minimal reconciliation: ensure each component has N replicas; restart on exit.
pub struct Supervisor {
    logs: SharedLogs,
    metrics: Arc<Metrics>,
    desired: tokio::sync::Mutex<BTreeMap<String, DesiredComponent>>,
    counts: tokio::sync::Mutex<HashMap<String, Arc<AtomicUsize>>>,
    tasks: tokio::sync::Mutex<HashMap<String, Vec<JoinHandle<()>>>>,
}

impl Supervisor {
    pub fn new(logs: SharedLogs, metrics: Arc<Metrics>) -> Self {
        Self {
            logs,
            metrics,
            desired: tokio::sync::Mutex::new(BTreeMap::new()),
            counts: tokio::sync::Mutex::new(HashMap::new()),
            tasks: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Restore desired state from disk on startup
    pub async fn restore_from_disk(
        &self,
        local_peer_id: Option<&str>,
        agent_roles: Option<&[String]>,
    ) -> anyhow::Result<()> {
        if let Some(manifest_toml) = load_desired_manifest() {
            info!("Restoring component state from persistent manifest");

            match toml::from_str::<Manifest>(&manifest_toml) {
                Ok(manifest) => {
                    let mut desired = BTreeMap::new();
                    let stage_dir = agent_data_dir().join("artifacts");

                    for (name, spec) in manifest.components {
                        if !spec.matches_target(local_peer_id, agent_roles) {
                            info!(component=%name, "Skipping restore: not targeted to this node");
                            continue;
                        }
                        if !spec.start {
                            info!(component=%name, "Manifest start=false; staged only");
                            continue;
                        }
                        // Resolve artifact path from cache using the pattern from handlers.rs
                        let artifact_path =
                            stage_dir.join(format!("{}-{}.wasm", name, &spec.sha256_hex[..16]));

                        if artifact_path.exists() {
                            // Verify the cached artifact still matches the expected hash
                            if let Ok(cached_bytes) = std::fs::read(&artifact_path) {
                                let cached_digest = sha256_hex(&cached_bytes);
                                if cached_digest == spec.sha256_hex {
                                    desired.insert(
                                        name.clone(),
                                        DesiredComponent {
                                            name: name.clone(),
                                            path: artifact_path.clone(),
                                            spec,
                                        },
                                    );
                                    info!(component=%name, path=%artifact_path.display(), "Restored component from cache");
                                } else {
                                    warn!(component=%name, expected=%spec.sha256_hex, actual=%cached_digest, "Cached artifact hash mismatch, skipping");
                                }
                            } else {
                                warn!(component=%name, path=%artifact_path.display(), "Failed to read cached artifact");
                            }
                        } else {
                            warn!(component=%name, path=%artifact_path.display(), "Cached artifact not found, component will be unavailable until re-deployed");
                        }
                    }

                    if !desired.is_empty() {
                        self.set_desired(desired.clone()).await;
                        info!(count=%desired.len(), "Successfully restored components from disk");

                        for name in desired.keys() {
                            push_log(
                                &self.logs,
                                name,
                                "restored from persistent state".to_string(),
                            )
                            .await;
                        }
                    } else {
                        info!("No components could be restored from cache");
                    }
                }
                Err(e) => {
                    warn!(error=%e, "Failed to parse persistent manifest, starting with empty state");
                }
            }
        } else {
            info!("No persistent manifest found, starting with empty state");
        }
        Ok(())
    }

    pub async fn set_desired(&self, desired: BTreeMap<String, DesiredComponent>) {
        let mut d = self.desired.lock().await;
        *d = desired;
        self.metrics.set_components_desired(d.len() as u64);
        // ensure counters exist
        let mut counts = self.counts.lock().await;
        for name in d.keys() {
            counts
                .entry(name.clone())
                .or_insert_with(|| Arc::new(AtomicUsize::new(0)));
        }
    }

    /// Upsert a single desired component specification and trigger reconciliation on next tick.
    pub async fn upsert_component(&self, desired: DesiredComponent) {
        let mut d = self.desired.lock().await;
        let name = desired.name.clone();
        d.insert(name.clone(), desired);
        self.metrics.set_components_desired(d.len() as u64);
        // ensure counter exists
        let mut counts = self.counts.lock().await;
        counts
            .entry(name)
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)));
    }

    pub fn spawn_reconcile(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut intv = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                intv.tick().await;
                self.reconcile_once().await;
            }
        });
    }

    /// Return a snapshot of desired components (cheap clone for gateway).
    pub async fn get_desired_snapshot(&self) -> BTreeMap<String, DesiredComponent> {
        self.desired.lock().await.clone()
    }

    /// Get one desired component by name, if present.
    pub async fn get_component(&self, name: &str) -> Option<DesiredComponent> {
        self.desired.lock().await.get(name).cloned()
    }

    async fn reconcile_once(&self) {
        let desired = self.desired.lock().await.clone();
        for (name, desired) in desired.into_iter() {
            let want = desired.spec.replicas.unwrap_or(1).max(1) as usize;
            let count_arc = {
                let mut counts = self.counts.lock().await;
                counts
                    .entry(name.clone())
                    .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
                    .clone()
            };
            let running = count_arc.load(Ordering::Relaxed);
            if running < want {
                let to_add = want.saturating_sub(running);
                for _ in 0..to_add {
                    self.launch_replica(desired.clone(), count_arc.clone())
                        .await;
                }
            }
        }
        // Note: minimal loop does not scale down replicas yet.
    }

    /// Clean up tasks for a given component
    pub async fn cleanup_component(&self, component_name: &str) {
        let mut tasks = self.tasks.lock().await;
        if let Some(handles) = tasks.remove(component_name) {
            for handle in handles {
                handle.abort();
            }
            info!(component=%component_name, "Component tasks cleaned up");
        }
        // Best-effort cleanup of component-level work directory (ephemeral). Any
        // per-replica subdirectories created for running replicas will be removed
        // on their normal exit path; if we forcibly stopped tasks, clear the tree.
        let work_root = crate::p2p::state::agent_data_dir()
            .join("work")
            .join("components")
            .join(component_name);
        if work_root.exists() {
            let _ = std::fs::remove_dir_all(&work_root);
        }
    }

    /// Clean up all running tasks
    #[allow(dead_code)]
    pub async fn cleanup_all(&self) {
        let mut tasks = self.tasks.lock().await;
        for (component, handles) in tasks.drain() {
            for handle in handles {
                handle.abort();
            }
            info!(component=%component, "All component tasks cleaned up");
        }
    }

    async fn launch_replica(&self, desired: DesiredComponent, count: Arc<AtomicUsize>) {
        let logs = self.logs.clone();
        let metrics = self.metrics.clone();
        let name = desired.name.clone();
        let path = desired.path.to_string_lossy().to_string();
        let mem = desired.spec.memory_max_mb.unwrap_or(64);
        let fuel = desired.spec.fuel.unwrap_or(5_000_000);
        let epoch = desired.spec.epoch_ms.unwrap_or(100);

        // Check if this is an HTTP component by inspecting the binary for HTTP handler exports
        if let Ok(wasm_bytes) = std::fs::read(&path) {
            // Simple string search in the binary for HTTP handler export signature
            let wasm_string = String::from_utf8_lossy(&wasm_bytes);
            if wasm_string.contains("wasi:http/incoming-handler") {
                info!(component=%name, "HTTP component detected - will be invoked on-demand via gateway");
                // For HTTP components, just mark as "running" but don't actually start a persistent process
                metrics.inc_components_running();
                count.fetch_add(1, Ordering::Relaxed);
                push_log(
                    &logs,
                    &name,
                    format!("HTTP component staged from {path}, ready for gateway invocation"),
                )
                .await;
                return;
            }
        }

        // Resolve per-replica work mount directory. Package 'work' mounts are resolved to
        // agent_data_dir()/work/components/{name}. Here we allocate a unique subdirectory
        // per replica and rewrite any matching mount host paths to that subdir. The subdir
        // is removed when the replica exits.
        let base_work_dir = crate::p2p::state::agent_data_dir()
            .join("work")
            .join("components")
            .join(&name);
        let mut effective_mounts: Option<Vec<common::MountSpec>> = desired.spec.mounts.clone();
        let mut replica_work_dir: Option<std::path::PathBuf> = None;
        if let Some(ref mut ms) = effective_mounts {
            let has_work_mount = ms.iter().any(|m| {
                let host_path = std::path::Path::new(&m.host);
                host_path.starts_with(&base_work_dir)
            });
            if has_work_mount {
                let replica_id = format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
                        .as_micros()
                );
                let dir = base_work_dir.join(replica_id);
                let _ = std::fs::create_dir_all(&dir);
                replica_work_dir = Some(dir.clone());
                for m in ms.iter_mut() {
                    let host_path = std::path::Path::new(&m.host);
                    if host_path.starts_with(&base_work_dir) {
                        m.host = dir.display().to_string();
                    }
                }
                if let Some(dir_log) = replica_work_dir.as_ref() {
                    push_log(
                        &logs,
                        &name,
                        format!("allocated work dir {}", dir_log.display()),
                    )
                    .await;
                }
            }
        }

        push_log(&logs, &name, format!("launching replica from {path}")).await;
        metrics.inc_components_running();
        count.fetch_add(1, Ordering::Relaxed);
        let name_run = name.clone();
        let mounts_for_run = effective_mounts.clone();
        let cleanup_work_dir = replica_work_dir.clone();
        let task_handle = tokio::spawn(async move {
            let res = run_wasm_module_with_limits(
                &path,
                &name_run,
                logs.clone(),
                mem,
                fuel,
                epoch,
                Some(metrics.clone()),
                mounts_for_run,
            )
            .await;
            if let Err(e) = &res {
                warn!(component=%name_run, error=%e, "replica crashed");
            }
            // Best-effort cleanup of per-replica work directory after exit
            if let Some(dir) = cleanup_work_dir.as_ref() {
                let _ = std::fs::remove_dir_all(dir);
            }
            // restart on crash by decrementing and letting next reconcile add back
            metrics.dec_components_running();
            metrics.inc_restarts_total();
            count.fetch_sub(1, Ordering::Relaxed);
        });

        // Track the task for cleanup
        let mut tasks = self.tasks.lock().await;
        tasks
            .entry(name.clone())
            .or_insert_with(Vec::new)
            .push(task_handle);

        info!(component=%name, "Component replica started");
    }
}
