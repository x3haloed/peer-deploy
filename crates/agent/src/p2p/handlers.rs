use anyhow::anyhow;
use base64::Engine;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

use crate::runner::run_wasm_module_with_limits;
use crate::supervisor::DesiredComponent;
use common::{sha256_hex, verify_bytes_ed25519, AgentUpgrade, Manifest, SignedManifest};

use super::metrics::{push_log, Metrics};
use super::metrics::SharedLogs;
use super::state::{
    agent_data_dir, load_state, load_trusted_owner, save_desired_manifest, save_state,
    save_trusted_owner,
};

/// Handle an ApplyManifest command from the network.
pub async fn handle_apply_manifest(
    tx: UnboundedSender<Result<String, String>>,
    signed: SignedManifest,
    logs: SharedLogs,
    metrics: std::sync::Arc<Metrics>,
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
                // Supervisor desired set will be set by the caller (p2p::run_agent) if wired
            }
            if let Err(e) = launch_components(staged, &signed.manifest_toml, logs.clone(), metrics.clone()).await {
                let _ = tx.send(Err(format!("launch error: {e}")));
                return;
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

/// Handle an UpgradeAgent command.
pub async fn handle_upgrade(tx: UnboundedSender<Result<String, String>>, pkg: AgentUpgrade) {
    use base64::Engine;

    // Decode signature and binary
    let sig = match base64::engine::general_purpose::STANDARD.decode(&pkg.signature_b64) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Err(format!("upgrade rejected (bad signature_b64: {e})")));
            return;
        }
    };
    let bin_bytes = match base64::engine::general_purpose::STANDARD.decode(&pkg.binary_b64) {
        Ok(b) => b,
        Err(e) => {
            let _ = tx.send(Err(format!("upgrade rejected (bad binary_b64: {e})")));
            return;
        }
    };

    // Verify signature and owner
    if verify_bytes_ed25519(&pkg.owner_pub_bs58, &bin_bytes, &sig).unwrap_or(false) {
        if let Some(trusted) = load_trusted_owner() {
            if trusted != pkg.owner_pub_bs58 {
                let _ = tx.send(Err("upgrade rejected (owner mismatch)".into()));
                return;
            }
        } else {
            save_trusted_owner(&pkg.owner_pub_bs58);
        }
    } else {
        let _ = tx.send(Err("upgrade rejected (sig)".into()));
        return;
    }

    // Verify digest
    let digest = sha256_hex(&bin_bytes);
    if digest != pkg.binary_sha256_hex {
        let _ = tx.send(Err("upgrade rejected (digest mismatch)".into()));
        return;
    }

    // Version monotonicity
    let mut state = load_state();
    if pkg.version <= state.agent_version {
        let _ = tx.send(Err(format!(
            "upgrade rejected (stale v{} <= v{})",
            pkg.version, state.agent_version
        )));
        return;
    }

    // Persist binary to a versioned path
    let bin_root = agent_data_dir().join("bin");
    if tokio::fs::create_dir_all(&bin_root).await.is_err() {
        let _ = tx.send(Err("upgrade rejected (bin dir create)".into()));
        return;
    }
    let versioned_path = bin_root.join(format!("realm-agent-v{}", pkg.version));
    match tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&versioned_path)
        .await
    {
        Ok(mut f) => {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = f.write_all(&bin_bytes).await {
                let _ = tx.send(Err(format!("upgrade rejected (write error: {e})")));
                return;
            }
            if let Err(e) = f.sync_all().await {
                let _ = tx.send(Err(format!("upgrade rejected (fsync file: {e})")));
                return;
            }
        }
        Err(e) => {
            let _ = tx.send(Err(format!("upgrade rejected (open error: {e})")));
            return;
        }
    }

    // Ensure executable bit on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&versioned_path, std::fs::Permissions::from_mode(0o755))
            .await;
    }

    // Fsync directory where the new binary lives (best-effort)
    #[cfg(unix)]
    {
        if let Ok(dir_file) = std::fs::File::open(&bin_root) {
            let _ = dir_file.sync_all();
        }
    }

    // Update a "current" symlink to the new version on Unix; ignore failures on non-Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let cur_link = bin_root.join("current");
        // Remove old link if present
        let _ = std::fs::remove_file(&cur_link);
        let _ = symlink(&versioned_path, &cur_link);
    }

    // Update state and spawn the new binary
    let previous = state.agent_version;
    state.previous_agent_version = previous;
    state.agent_version = pkg.version;
    save_state(&state);

    let ok_msg = format!("upgrade accepted v{} (prev v{})", pkg.version, previous);
    let _ = tx.send(Ok(ok_msg));

    // Spawn new process from the freshly written binary and exit this one.
    // Prefer versioned path to avoid rename-on-Windows issues.
    let _ = std::process::Command::new(&versioned_path)
        .args(std::env::args().skip(1))
        .spawn();
    // Give the status publisher a moment to flush before exit
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::process::exit(0);
}

async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("file:") {
        let path = std::path::Path::new(rest);
        return Ok(tokio::fs::read(path).await?);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let res = reqwest::get(url).await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("fetch {}: {}", url, status));
        }
        let bytes = res.bytes().await?;
        return Ok(bytes.to_vec());
    }
    Err(anyhow!("unsupported source: {}", url))
}

async fn verify_and_stage_artifacts(
    manifest_toml: &str,
) -> anyhow::Result<std::collections::BTreeMap<String, std::path::PathBuf>> {
    let manifest: Manifest = toml::from_str(manifest_toml)?;
    let mut staged = std::collections::BTreeMap::new();
    let stage_dir = agent_data_dir().join("artifacts");
    tokio::fs::create_dir_all(&stage_dir).await.ok();
    for (name, comp) in manifest.components.iter() {
        let bytes = fetch_bytes(&comp.source).await?;
        let digest = sha256_hex(&bytes);
        if digest != comp.sha256_hex {
            return Err(anyhow!("component {} digest mismatch", name));
        }
        let file_path = stage_dir.join(format!("{}-{}.wasm", name, &digest[..16]));
        if !file_path.exists() {
            tokio::fs::write(&file_path, &bytes).await?;
        }
        staged.insert(name.clone(), file_path);
    }
    Ok(staged)
}

async fn launch_components(
    staged: std::collections::BTreeMap<String, std::path::PathBuf>,
    manifest_toml: &str,
    logs: SharedLogs,
    metrics: std::sync::Arc<Metrics>,
) -> anyhow::Result<()> {
    let manifest: Manifest = toml::from_str(manifest_toml)?;
    for (name, path) in staged {
        if let Some(spec) = manifest.components.get(&name) {
            let mem = spec.memory_max_mb.unwrap_or(64);
            let fuel = spec.fuel.unwrap_or(5_000_000);
            let epoch = spec.epoch_ms.unwrap_or(100);
            let p = path.to_string_lossy().to_string();
            let logs = logs.clone();
            let n = name.clone();
            let m = metrics.clone();
            tokio::spawn(async move {
                // mark as running while the component task is active
                m.inc_components_running();
                let res = run_wasm_module_with_limits(&p, &n, logs.clone(), mem, fuel, epoch).await;
                if let Err(e) = res { warn!(component=%n, error=%e, "component run failed"); }
                // decrement when it exits
                m.dec_components_running();
            });
        }
    }
    Ok(())
}
