use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

use tracing::{info, warn};

use crate::p2p::metrics::{push_log, Metrics, SharedLogs};
use crate::runner::run_wasm_module_with_limits;
use common::ComponentSpec;

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
}

impl Supervisor {
    pub fn new(logs: SharedLogs, metrics: Arc<Metrics>) -> Self {
        Self {
            logs,
            metrics,
            desired: tokio::sync::Mutex::new(BTreeMap::new()),
            counts: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub async fn set_desired(&self, desired: BTreeMap<String, DesiredComponent>) {
        let mut d = self.desired.lock().await;
        *d = desired;
        self.metrics.set_components_desired(d.len() as u64);
        // ensure counters exist
        let mut counts = self.counts.lock().await;
        for name in d.keys() {
            counts.entry(name.clone()).or_insert_with(|| Arc::new(AtomicUsize::new(0)));
        }
    }

    pub async fn desired_len(&self) -> usize { self.desired.lock().await.len() }

    pub fn spawn_reconcile(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut intv = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                intv.tick().await;
                self.reconcile_once().await;
            }
        });
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
                    self.launch_replica(desired.clone(), count_arc.clone()).await;
                }
            }
        }
        // Note: minimal loop does not scale down replicas yet.
    }

    async fn launch_replica(&self, desired: DesiredComponent, count: Arc<AtomicUsize>) {
        let logs = self.logs.clone();
        let metrics = self.metrics.clone();
        let name = desired.name.clone();
        let path = desired.path.to_string_lossy().to_string();
        let mem = desired.spec.memory_max_mb.unwrap_or(64);
        let fuel = desired.spec.fuel.unwrap_or(5_000_000);
        let epoch = desired.spec.epoch_ms.unwrap_or(100);
        push_log(&logs, &name, format!("launching replica from {path}")).await;
        metrics.inc_components_running();
        count.fetch_add(1, Ordering::Relaxed);
        let name_run = name.clone();
        tokio::spawn(async move {
            let res = run_wasm_module_with_limits(&path, &name_run, logs.clone(), mem, fuel, epoch, Some(metrics.clone())).await;
            if let Err(e) = &res { warn!(component=%name_run, error=%e, "replica crashed"); }
            // restart on crash by decrementing and letting next reconcile add back
            metrics.dec_components_running();
            metrics.inc_restarts_total();
            count.fetch_sub(1, Ordering::Relaxed);
        });
        info!(component=%name, "replica started");
    }
}


