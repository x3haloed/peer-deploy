use anyhow::anyhow;
use base64::Engine;
use tokio::sync::mpsc::UnboundedSender;

// use crate::runner::run_wasm_module_with_limits; // supervisor handles launching
use crate::supervisor::{DesiredComponent, Supervisor};
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
    supervisor: std::sync::Arc<Supervisor>,
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

// legacy ad-hoc launcher retained for reference; superseded by supervisor

/// Return a short string like "linux/x86_64" for the current process.
fn host_platform_string() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Quick header sniff of `bin` to determine its OS and arch and check against the host.
/// - Accepts: ELF(linux), Mach-O(macos, incl. universal/fat), PE(windows)
/// - Returns Ok(with bin description) if matches; Err(reason) otherwise.
fn binary_target_matches_host(bin: &[u8]) -> Result<String, String> {
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;

    match detect_binary_os(bin) {
        Some(BinaryOs::Linux) => {
            if host_os != "linux" { return Err(format!("host {} but bin linux", host_os)); }
            match detect_elf_arch(bin) {
                Some(a) if arch_matches(&a, host_arch) => Ok(format!("linux/{}", a)),
                Some(a) => Err(format!("host {}/{} but bin linux/{}", host_os, host_arch, a)),
                None => Err("unrecognized ELF arch".into()),
            }
        }
        Some(BinaryOs::MacOs) => {
            if host_os != "macos" { return Err(format!("host {} but bin macos", host_os)); }
            match detect_macho_arches(bin) {
                Some(arches) => {
                    if arches.iter().any(|a| arch_matches(a, host_arch)) {
                        Ok(format!("macos/{}", arches.join("|")))
                    } else {
                        Err(format!("host {}/{} but bin macos/{}", host_os, host_arch, arches.join("|")))
                    }
                }
                None => Err("unrecognized Mach-O arch".into()),
            }
        }
        Some(BinaryOs::Windows) => {
            if host_os != "windows" { return Err(format!("host {} but bin windows", host_os)); }
            match detect_pe_arch(bin) {
                Some(a) if arch_matches(&a, host_arch) => Ok(format!("windows/{}", a)),
                Some(a) => Err(format!("host {}/{} but bin windows/{}", host_os, host_arch, a)),
                None => Err("unrecognized PE arch".into()),
            }
        }
        None => Err("unknown binary format".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryOs { Linux, MacOs, Windows }

fn detect_binary_os(bin: &[u8]) -> Option<BinaryOs> {
    if bin.len() >= 4 {
        let m4 = &bin[0..4];
        if m4 == [0x7F, b'E', b'L', b'F'] { return Some(BinaryOs::Linux); }
        if m4 == [0x4D, 0x5A, 0x90, 0x00] || &bin[0..2] == b"MZ" { return Some(BinaryOs::Windows); }
        // Mach-O thin and fat magics
        if m4 == [0xFE, 0xED, 0xFA, 0xCE] || m4 == [0xFE, 0xED, 0xFA, 0xCF]
            || m4 == [0xCE, 0xFA, 0xED, 0xFE] || m4 == [0xCF, 0xFA, 0xED, 0xFE]
            || m4 == [0xCA, 0xFE, 0xBA, 0xBE] || m4 == [0xBE, 0xBA, 0xFE, 0xCA]
        {
            return Some(BinaryOs::MacOs);
        }
    }
    None
}

fn arch_matches(bin_arch: &str, host_arch: &str) -> bool {
    // Normalize some aliases
    match (bin_arch, host_arch) {
        ("x86_64", "x86_64") => true,
        ("aarch64", "aarch64") => true,
        ("arm", "arm") => true,
        ("x86", "x86") => true,
        ("riscv64", "riscv64") => true,
        // Apple Rosetta 2 does NOT allow running arm64 binaries on x86_64 or vice versa; do not cross-accept.
        _ => false,
    }
}

fn detect_elf_arch(bin: &[u8]) -> Option<String> {
    if bin.len() < 20 || &bin[0..4] != [0x7F, b'E', b'L', b'F'] { return None; }
    let ei_data = bin[5];
    let is_le = ei_data == 1; // 1=little, 2=big
    let em_off = 18usize;
    if bin.len() < em_off + 2 { return None; }
    let em = if is_le {
        u16::from_le_bytes([bin[em_off], bin[em_off + 1]])
    } else {
        u16::from_be_bytes([bin[em_off], bin[em_off + 1]])
    };
    let arch = match em {
        62 => "x86_64",
        183 => "aarch64",
        3 => "x86",
        40 => "arm",
        243 => "riscv64",
        _ => return None,
    }.to_string();
    Some(arch)
}

fn detect_macho_arches(bin: &[u8]) -> Option<Vec<String>> {
    if bin.len() < 8 { return None; }
    let m4 = &bin[0..4];
    // FAT/universal
    if m4 == [0xCA, 0xFE, 0xBA, 0xBE] || m4 == [0xBE, 0xBA, 0xFE, 0xCA]
        || m4 == [0xCA, 0xFE, 0xBA, 0xBF] || m4 == [0xBF, 0xBA, 0xFE, 0xCA]
    {
        let be = m4 == [0xCA, 0xFE, 0xBA, 0xBE] || m4 == [0xCA, 0xFE, 0xBA, 0xBF];
        if bin.len() < 12 { return None; }
        let nfat_arch = if be {
            u32::from_be_bytes([bin[4], bin[5], bin[6], bin[7]])
        } else {
            u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]])
        } as usize;
        let mut arches = Vec::new();
        let mut off = 8usize;
        // fat_arch is 20 bytes; fat_arch_64 is 32 bytes. Choose based on magic.
        let rec_size = if m4 == [0xCA, 0xFE, 0xBA, 0xBF] || m4 == [0xBF, 0xBA, 0xFE, 0xCA] { 32 } else { 20 };
        for _ in 0..nfat_arch {
            if bin.len() < off + rec_size { break; }
            let cputype = if be {
                u32::from_be_bytes([bin[off], bin[off+1], bin[off+2], bin[off+3]])
            } else {
                u32::from_le_bytes([bin[off], bin[off+1], bin[off+2], bin[off+3]])
            };
            if let Some(a) = macho_cputype_to_arch(cputype) { arches.push(a.to_string()); }
            off += rec_size; // skip fat_arch or fat_arch_64
        }
        if arches.is_empty() { None } else { Some(arches) }
    } else if m4 == [0xFE, 0xED, 0xFA, 0xCE] || m4 == [0xFE, 0xED, 0xFA, 0xCF]
        || m4 == [0xCE, 0xFA, 0xED, 0xFE] || m4 == [0xCF, 0xFA, 0xED, 0xFE] {
        let be = m4 == [0xFE, 0xED, 0xFA, 0xCE] || m4 == [0xFE, 0xED, 0xFA, 0xCF];
        if bin.len() < 8 { return None; }
        let cputype = if be {
            u32::from_be_bytes([bin[4], bin[5], bin[6], bin[7]])
        } else {
            u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]])
        };
        macho_cputype_to_arch(cputype).map(|s| vec![s.to_string()])
    } else {
        None
    }
}

fn macho_cputype_to_arch(cputype: u32) -> Option<&'static str> {
    const CPU_ARCH_ABI64: u32 = 0x0100_0000;
    let base = cputype & 0x00FF_FFFF;
    let is64 = (cputype & CPU_ARCH_ABI64) != 0;
    match (base, is64) {
        (7, true) => Some("x86_64"), // CPU_TYPE_X86 | 64
        (7, false) => Some("x86"),
        (12, true) => Some("aarch64"), // CPU_TYPE_ARM | 64
        (12, false) => Some("arm"),
        _ => None,
    }
}

fn detect_pe_arch(bin: &[u8]) -> Option<String> {
    if bin.len() < 0x40 || &bin[0..2] != b"MZ" { return None; }
    let pe_off = u32::from_le_bytes([bin[0x3C], bin[0x3D], bin[0x3E], bin[0x3F]]) as usize;
    if bin.len() < pe_off + 6 { return None; }
    if &bin[pe_off..pe_off+4] != b"PE\0\0" { return None; }
    let machine = u16::from_le_bytes([bin[pe_off + 4], bin[pe_off + 5]]);
    let arch = match machine {
        0x8664 => "x86_64",
        0x014c => "x86",
        0xAA64 => "aarch64",
        0x01c0 => "arm",
        _ => return None,
    }.to_string();
    Some(arch)
}
