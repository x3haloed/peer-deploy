use base64::Engine;
use tokio::sync::mpsc::UnboundedSender;

use crate::supervisor::DesiredComponent;
use common::{verify_bytes_ed25519, Manifest, SignedManifest};

use super::super::metrics::{push_log, Metrics};
use super::super::state::{load_state, load_trusted_owner, save_desired_manifest, save_state, save_trusted_owner};
use super::util::verify_and_stage_artifacts;

/// Handle an ApplyManifest command from the network.
pub async fn handle_apply_manifest(
    tx: UnboundedSender<Result<String, String>>,
    signed: SignedManifest,
    logs: super::super::metrics::SharedLogs,
    metrics: std::sync::Arc<Metrics>,
    supervisor: std::sync::Arc<crate::supervisor::Supervisor>,
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
    // Verify and stage artifacts, then launch and persist version
    match verify_and_stage_artifacts(&signed.manifest_toml).await {
        Ok(staged) => {
            // Update desired components count from manifest
            if let Ok(mf) = toml::from_str::<Manifest>(&signed.manifest_toml) {
                metrics.set_components_desired(mf.components.len() as u64);
                // Persist desired manifest
                save_desired_manifest(&signed.manifest_toml);
                // Build desired set for supervisor
                let mut desired: std::collections::BTreeMap<String, DesiredComponent> = Default::default();
                for (name, spec) in mf.components.iter() {
                    if let Some(path) = staged.get(name) {
                        desired.insert(
                            name.clone(),
                            DesiredComponent { name: name.clone(), path: path.clone(), spec: spec.clone() },
                        );
                    }
                }
                // Update supervisor desired set
                supervisor.set_desired(desired).await;
            }
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
        Err(e) => {
            let _ = tx.send(Err(format!("manifest rejected (digest): {e}")));
            push_log(&logs, "apply", format!("manifest rejected (digest): {e}")).await;
        }
    }
}


