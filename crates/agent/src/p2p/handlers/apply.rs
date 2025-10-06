use base64::Engine;
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;

use crate::supervisor::DesiredComponent;
use common::{verify_bytes_ed25519, Manifest, SignedManifest};

use super::super::metrics::{push_log, Metrics};
use super::super::state::{
    load_state, load_trusted_owner, save_desired_manifest, save_state, save_trusted_owner,
};
use super::util::verify_and_stage_artifacts;

/// Handle an ApplyManifest command from the network.
pub async fn handle_apply_manifest(
    tx: UnboundedSender<Result<String, String>>,
    signed: SignedManifest,
    logs: super::super::metrics::SharedLogs,
    metrics: std::sync::Arc<Metrics>,
    supervisor: std::sync::Arc<crate::supervisor::Supervisor>,
    local_peer_id: String,
    agent_roles: Vec<String>,
) {
    // Signature check
    let sig = match base64::engine::general_purpose::STANDARD.decode(&signed.signature_b64) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Err(format!("bad signature_b64: {e}")));
            return;
        }
    };
    let ok = verify_bytes_ed25519(
        &signed.owner_pub_bs58,
        signed.manifest_toml.as_bytes(),
        &sig,
    )
    .unwrap_or(false);
    if !ok {
        let _ = tx.send(Err("manifest rejected (sig)".into()));
        return;
    }
    // TOFU
    if let Some(trusted) = load_trusted_owner() {
        if trusted != signed.owner_pub_bs58 {
            let _ = tx.send(Err("manifest rejected (owner mismatch)".into()));
            return;
        }
    } else {
        save_trusted_owner(&signed.owner_pub_bs58);
    }
    // Monotonic version
    let state = load_state();
    if state.manifest_version >= signed.version {
        let _ = tx.send(Err(format!(
            "manifest rejected (stale v{} <= v{})",
            signed.version, state.manifest_version
        )));
        return;
    }
    // Parse manifest once so we can filter per-node
    let manifest = match toml::from_str::<Manifest>(&signed.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(Err(format!("manifest rejected (parse): {e}")));
            push_log(&logs, "apply", format!("manifest parse error: {e}")).await;
            return;
        }
    };

    // Filter components that apply to this node based on selectors
    let mut applicable = BTreeMap::new();
    let peer_id_ref = local_peer_id.as_str();
    let role_slice = agent_roles.as_slice();
    for (name, spec) in manifest.components.iter() {
        if spec.matches_target(Some(peer_id_ref), Some(role_slice)) {
            applicable.insert(name.clone(), spec.clone());
        } else {
            push_log(&logs, "apply", format!("skipping {} (not targeted)", name)).await;
        }
    }

    let filtered_manifest = Manifest {
        components: applicable.clone(),
    };

    let staged = if filtered_manifest.components.is_empty() {
        BTreeMap::new()
    } else {
        match verify_and_stage_artifacts(&filtered_manifest).await {
            Ok(staged) => staged,
            Err(e) => {
                let _ = tx.send(Err(format!("manifest rejected (digest): {e}")));
                push_log(&logs, "apply", format!("manifest rejected (digest): {e}")).await;
                return;
            }
        }
    };

    let desired_count = applicable.values().filter(|spec| spec.start).count() as u64;
    metrics.set_components_desired(desired_count);

    save_desired_manifest(&signed.manifest_toml);

    // Build desired set for supervisor using staged artifacts
    let mut desired: BTreeMap<String, DesiredComponent> = BTreeMap::new();
    for (name, spec) in applicable.iter() {
        if !spec.start {
            push_log(
                &logs,
                name,
                "manifest staged with start=false (not scheduling)".to_string(),
            )
            .await;
            continue;
        }
        if let Some(path) = staged.get(name) {
            desired.insert(
                name.clone(),
                DesiredComponent {
                    name: name.clone(),
                    path: path.clone(),
                    spec: spec.clone(),
                },
            );
        } else {
            push_log(
                &logs,
                "apply",
                format!("missing staged artifact for {}", name),
            )
            .await;
        }
    }

    supervisor.set_desired(desired).await;

    let mut state2 = load_state();
    state2.manifest_version = signed.version;
    save_state(&state2);
    let _ = tx.send(Ok(format!("manifest accepted v{}", signed.version)));
    push_log(
        &logs,
        "apply",
        format!("manifest accepted v{}", signed.version),
    )
    .await;
}
