use anyhow::anyhow;
use common::Manifest;

use super::super::state::agent_data_dir;

pub(crate) async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
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

pub(super) async fn verify_and_stage_artifacts(
    manifest: &Manifest,
) -> anyhow::Result<std::collections::BTreeMap<String, std::path::PathBuf>> {
    use common::sha256_hex;
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

pub(super) fn host_platform_string() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryOs {
    Linux,
    MacOs,
    Windows,
}

pub(super) fn binary_target_matches_host(bin: &[u8]) -> Result<String, String> {
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;

    match detect_binary_os(bin) {
        Some(BinaryOs::Linux) => {
            if host_os != "linux" {
                return Err(format!("host {} but bin linux", host_os));
            }
            match detect_elf_arch(bin) {
                Some(a) if arch_matches(&a, host_arch) => Ok(format!("linux/{}", a)),
                Some(a) => Err(format!(
                    "host {}/{} but bin linux/{}",
                    host_os, host_arch, a
                )),
                None => Err("unrecognized ELF arch".into()),
            }
        }
        Some(BinaryOs::MacOs) => {
            if host_os != "macos" {
                return Err(format!("host {} but bin macos", host_os));
            }
            match detect_macho_arches(bin) {
                Some(arches) => {
                    if arches.iter().any(|a| arch_matches(a, host_arch)) {
                        Ok(format!("macos/{}", arches.join("|")))
                    } else {
                        Err(format!(
                            "host {}/{} but bin macos/{}",
                            host_os,
                            host_arch,
                            arches.join("|")
                        ))
                    }
                }
                None => Err("unrecognized Mach-O arch".into()),
            }
        }
        Some(BinaryOs::Windows) => {
            if host_os != "windows" {
                return Err(format!("host {} but bin windows", host_os));
            }
            match detect_pe_arch(bin) {
                Some(a) if arch_matches(&a, host_arch) => Ok(format!("windows/{}", a)),
                Some(a) => Err(format!(
                    "host {}/{} but bin windows/{}",
                    host_os, host_arch, a
                )),
                None => Err("unrecognized PE arch".into()),
            }
        }
        None => Err("unknown binary format".into()),
    }
}

fn detect_binary_os(bin: &[u8]) -> Option<BinaryOs> {
    if bin.len() >= 4 {
        let m4 = &bin[0..4];
        if m4 == [0x7F, b'E', b'L', b'F'] {
            return Some(BinaryOs::Linux);
        }
        if m4 == [0x4D, 0x5A, 0x90, 0x00] || &bin[0..2] == b"MZ" {
            return Some(BinaryOs::Windows);
        }
        // Mach-O thin and fat magics
        if m4 == [0xFE, 0xED, 0xFA, 0xCE]
            || m4 == [0xFE, 0xED, 0xFA, 0xCF]
            || m4 == [0xCE, 0xFA, 0xED, 0xFE]
            || m4 == [0xCF, 0xFA, 0xED, 0xFE]
            || m4 == [0xCA, 0xFE, 0xBA, 0xBE]
            || m4 == [0xBE, 0xBA, 0xFE, 0xCA]
        {
            return Some(BinaryOs::MacOs);
        }
    }
    None
}

fn arch_matches(bin_arch: &str, host_arch: &str) -> bool {
    match (bin_arch, host_arch) {
        ("x86_64", "x86_64") => true,
        ("aarch64", "aarch64") => true,
        ("arm", "arm") => true,
        ("x86", "x86") => true,
        ("riscv64", "riscv64") => true,
        _ => false,
    }
}

fn detect_elf_arch(bin: &[u8]) -> Option<String> {
    if bin.len() < 20 || &bin[0..4] != [0x7F, b'E', b'L', b'F'] {
        return None;
    }
    let ei_data = bin[5];
    let is_le = ei_data == 1; // 1=little, 2=big
    let em_off = 18usize;
    if bin.len() < em_off + 2 {
        return None;
    }
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
    }
    .to_string();
    Some(arch)
}

fn detect_macho_arches(bin: &[u8]) -> Option<Vec<String>> {
    if bin.len() < 8 {
        return None;
    }
    let m4 = &bin[0..4];
    // FAT/universal
    if m4 == [0xCA, 0xFE, 0xBA, 0xBE]
        || m4 == [0xBE, 0xBA, 0xFE, 0xCA]
        || m4 == [0xCA, 0xFE, 0xBA, 0xBF]
        || m4 == [0xBF, 0xBA, 0xFE, 0xCA]
    {
        let be = m4 == [0xCA, 0xFE, 0xBA, 0xBE] || m4 == [0xCA, 0xFE, 0xBA, 0xBF];
        if bin.len() < 12 {
            return None;
        }
        let nfat_arch = if be {
            u32::from_be_bytes([bin[4], bin[5], bin[6], bin[7]])
        } else {
            u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]])
        } as usize;
        let mut arches = Vec::new();
        let mut off = 8usize;
        // fat_arch is 20 bytes; fat_arch_64 is 32 bytes. Choose based on magic.
        let rec_size = if m4 == [0xCA, 0xFE, 0xBA, 0xBF] || m4 == [0xBF, 0xBA, 0xFE, 0xCA] {
            32
        } else {
            20
        };
        for _ in 0..nfat_arch {
            if bin.len() < off + rec_size {
                break;
            }
            let cputype = if be {
                u32::from_be_bytes([bin[off], bin[off + 1], bin[off + 2], bin[off + 3]])
            } else {
                u32::from_le_bytes([bin[off], bin[off + 1], bin[off + 2], bin[off + 3]])
            };
            if let Some(a) = macho_cputype_to_arch(cputype) {
                arches.push(a.to_string());
            }
            off += rec_size; // skip fat_arch or fat_arch_64
        }
        if arches.is_empty() {
            None
        } else {
            Some(arches)
        }
    } else if m4 == [0xFE, 0xED, 0xFA, 0xCE]
        || m4 == [0xFE, 0xED, 0xFA, 0xCF]
        || m4 == [0xCE, 0xFA, 0xED, 0xFE]
        || m4 == [0xCF, 0xFA, 0xED, 0xFE]
    {
        let be = m4 == [0xFE, 0xED, 0xFA, 0xCE] || m4 == [0xFE, 0xED, 0xFA, 0xCF];
        if bin.len() < 8 {
            return None;
        }
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
    if bin.len() < 0x40 || &bin[0..2] != b"MZ" {
        return None;
    }
    let pe_off = u32::from_le_bytes([bin[0x3C], bin[0x3D], bin[0x3E], bin[0x3F]]) as usize;
    if bin.len() < pe_off + 6 {
        return None;
    }
    if &bin[pe_off..pe_off + 4] != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes([bin[pe_off + 4], bin[pe_off + 5]]);
    let arch = match machine {
        0x8664 => "x86_64",
        0x014c => "x86",
        0xAA64 => "aarch64",
        0x01c0 => "arm",
        _ => return None,
    }
    .to_string();
    Some(arch)
}
