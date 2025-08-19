use std::time::Duration;

use anyhow::Context;
use base64::Engine;

use common::{serialize_message, Command, OwnerKeypair, AgentUpgrade, sign_bytes_ed25519, sha256_hex};

use super::util::{new_swarm, mdns_warmup, owner_dir};

pub async fn upgrade_multi(
    bins: Vec<String>,
    file: Option<String>,
    target_platform: Option<String>,
    all_platforms: bool,
    version: u64,
    target_peers: Vec<String>,
    target_tags: Vec<String>,
) -> anyhow::Result<()> {
    // Interpret inputs:
    // - If --bin provided: entries of form "<plat>=<path>" or just "<path>".
    // - Else: fall back to single file/target_platform.
    if bins.is_empty() {
        let file = file.ok_or_else(|| anyhow::anyhow!("--file is required if no --bin provided"))?;
        return upgrade(file, version, target_platform, target_peers, target_tags).await;
    }

    for entry in bins {
        let (plat_opt, path) = if let Some((p, f)) = entry.split_once('=') {
            (Some(p.trim().to_string()), f.trim().to_string())
        } else {
            (None, entry)
        };
        // If --all-platforms is false and a platform is specified on the flag, respect it; otherwise use split or detect
        let plat = if all_platforms {
            plat_opt
        } else {
            plat_opt.or_else(|| target_platform.clone())
        };
        upgrade(path, version, plat, target_peers.clone(), target_tags.clone()).await?;
        // Small delay to avoid overwhelming the gossip topic with large messages simultaneously
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

pub async fn upgrade(
    file: String,
    version: u64,
    target_platform: Option<String>,
    target_peers: Vec<String>,
    target_tags: Vec<String>,
) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;

    mdns_warmup(&mut swarm).await;

    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    let bin_bytes = tokio::fs::read(&file).await?;
    let digest = sha256_hex(&bin_bytes);
    let sig = sign_bytes_ed25519(&kp.private_hex, &bin_bytes)?;
    // If platform not provided, attempt to detect from binary headers (best-effort)
    let platform_detected = target_platform.or_else(|| detect_binary_platform(&bin_bytes));
    let pkg = AgentUpgrade {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        version,
        target_platform: platform_detected,
        target_peer_ids: target_peers,
        target_tags,
        binary_sha256_hex: digest,
        binary_b64: base64::engine::general_purpose::STANDARD.encode(&bin_bytes),
        signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
    };
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::UpgradeAgent(pkg)))?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

// ------- Binary platform detection (best-effort, mirrors agent checks) -------

fn detect_binary_platform(bin: &[u8]) -> Option<String> {
    match detect_binary_os(bin)? {
        BinaryOs::Linux => {
            let arch = detect_elf_arch(bin)?;
            Some(format!("linux/{}", arch))
        }
        BinaryOs::MacOs => {
            match detect_macho_arches(bin) {
                Some(arches) if arches.len() == 1 => Some(format!("macos/{}", arches[0])),
                // Universal/fat: serve to all mac targets; let agents self-select
                _ => None,
            }
        }
        BinaryOs::Windows => {
            let arch = detect_pe_arch(bin)?;
            Some(format!("windows/{}", arch))
        }
    }
}

#[derive(Debug, Clone, Copy)]
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

