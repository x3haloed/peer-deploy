use tokio::sync::mpsc::UnboundedSender;

use common::{sha256_hex, verify_bytes_ed25519, AgentUpgrade};

use super::super::metrics::push_log;
use super::super::metrics::SharedLogs;
use super::super::state::{agent_data_dir, load_state, load_trusted_owner, save_state, save_trusted_owner};
use super::util::{binary_target_matches_host, host_platform_string};

/// Handle an UpgradeAgent command.
pub async fn handle_upgrade(
    tx: UnboundedSender<Result<String, String>>,
    pkg: AgentUpgrade,
    logs: SharedLogs,
) {
    use base64::Engine;

    // Decode signature and binary
    let sig = match base64::engine::general_purpose::STANDARD.decode(&pkg.signature_b64) {
        Ok(s) => s,
        Err(e) => {
            push_log(&logs, "upgrade", format!("upgrade rejected (bad signature_b64: {e})")).await;
            let _ = tx.send(Err(format!("upgrade rejected (bad signature_b64: {e})")));
            return;
        }
    };
    let bin_bytes = match base64::engine::general_purpose::STANDARD.decode(&pkg.binary_b64) {
        Ok(b) => b,
        Err(e) => {
            push_log(&logs, "upgrade", format!("upgrade rejected (bad binary_b64: {e})")).await;
            let _ = tx.send(Err(format!("upgrade rejected (bad binary_b64: {e})")));
            return;
        }
    };

    // Verify signature and owner
    if verify_bytes_ed25519(&pkg.owner_pub_bs58, &bin_bytes, &sig).unwrap_or(false) {
        if let Some(trusted) = load_trusted_owner() {
            if trusted != pkg.owner_pub_bs58 {
                push_log(&logs, "upgrade", "upgrade rejected (owner mismatch)" ).await;
                let _ = tx.send(Err("upgrade rejected (owner mismatch)".into()));
                return;
            }
        } else {
            push_log(&logs, "upgrade", "TOFU: trusting owner for upgrade" ).await;
            save_trusted_owner(&pkg.owner_pub_bs58);
        }
    } else {
        push_log(&logs, "upgrade", "upgrade rejected (sig)" ).await;
        let _ = tx.send(Err("upgrade rejected (sig)".into()));
        return;
    }

    // Verify digest
    let digest = sha256_hex(&bin_bytes);
    if digest != pkg.binary_sha256_hex {
        push_log(&logs, "upgrade", "upgrade rejected (digest mismatch)" ).await;
        let _ = tx.send(Err("upgrade rejected (digest mismatch)".into()));
        return;
    }
    push_log(&logs, "upgrade", format!("verified signature and digest sha256={}", &digest[..16])).await;

    // Verify binary target matches host OS/arch via header sniff (no external deps)
    match binary_target_matches_host(&bin_bytes) {
        Ok(desc) => {
            push_log(&logs, "upgrade", format!(
                "binary target OK (host {} / bin {})",
                host_platform_string(),
                desc
            )).await;
        }
        Err(err) => {
            push_log(&logs, "upgrade", format!("upgrade rejected (target mismatch: {})", err)).await;
            let _ = tx.send(Err(format!("upgrade rejected (target mismatch: {})", err)));
            return;
        }
    }

    // If the package specifies a target platform, enforce it explicitly here as well
    if let Some(ref plat) = pkg.target_platform {
        let host = host_platform_string();
        if &host != plat {
            push_log(&logs, "upgrade", format!("upgrade rejected (platform {} != {})", host, plat)).await;
            let _ = tx.send(Err(format!("upgrade rejected (platform {} != {})", host, plat)));
            return;
        }
    }

    // Version monotonicity
    let mut state = load_state();
    if pkg.version <= state.agent_version {
        push_log(&logs, "upgrade", format!(
            "upgrade rejected (stale v{} <= v{})",
            pkg.version, state.agent_version
        )).await;
        let _ = tx.send(Err(format!(
            "upgrade rejected (stale v{} <= v{})",
            pkg.version, state.agent_version
        )));
        return;
    }

    // Persist binary to a versioned path
    let bin_root = agent_data_dir().join("bin");
    if tokio::fs::create_dir_all(&bin_root).await.is_err() {
        push_log(&logs, "upgrade", "upgrade rejected (bin dir create)" ).await;
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
                push_log(&logs, "upgrade", format!("upgrade rejected (write error: {e})")).await;
                let _ = tx.send(Err(format!("upgrade rejected (write error: {e})")));
                return;
            }
            if let Err(e) = f.sync_all().await {
                push_log(&logs, "upgrade", format!("upgrade rejected (fsync file: {e})")).await;
                let _ = tx.send(Err(format!("upgrade rejected (fsync file: {e})")));
                return;
            }
            push_log(&logs, "upgrade", format!("wrote versioned binary to {}", versioned_path.display())).await;
        }
        Err(e) => {
            push_log(&logs, "upgrade", format!("upgrade rejected (open error: {e})")).await;
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
        push_log(&logs, "upgrade", "set executable bit on new binary").await;
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
        push_log(&logs, "upgrade", format!("updated symlink {} -> {}", cur_link.display(), versioned_path.display())).await;
    }

    // Update state and spawn the new binary
    let previous = state.agent_version;
    state.previous_agent_version = previous;
    state.agent_version = pkg.version;
    save_state(&state);

    let ok_msg = format!("upgrade accepted v{} (prev v{})", pkg.version, previous);
    let _ = tx.send(Ok(ok_msg));
    push_log(&logs, "upgrade", format!("upgrade accepted v{} (prev v{})", pkg.version, previous)).await;

    // Spawn new process from the freshly written binary and exit this one.
    // Prefer versioned path to avoid rename-on-Windows issues.
    push_log(&logs, "upgrade", format!("spawning new process: {}", versioned_path.display())).await;
    let spawn_res = std::process::Command::new(&versioned_path)
        .args(std::env::args().skip(1))
        .spawn();
    if spawn_res.is_err() {
        push_log(&logs, "upgrade", "spawn failed; retaining old process and previous version").await;
        // Roll back visible version to previous
        let mut s = load_state();
        s.agent_version = previous;
        save_state(&s);
        let _ = tx.send(Err("upgrade rejected (spawn failed)".into()));
        return;
    }
    push_log(&logs, "upgrade", "exiting old process").await;
    // Give the status publisher a moment to flush before exit
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::process::exit(0);
}


