use super::types::*;
use crate::supervisor::DesiredComponent;
use axum::{
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use common::{sha256_hex, ComponentSpec, PackageManifest};
use serde_json::json;
use std::io::Cursor;
use std::io::Read;

/// Upload and deploy a .realm package (zip) containing a package manifest and assets.
/// Expects multipart fields: file (the .realm/.zip), name (optional override)
pub async fn api_deploy_package_multipart(
    State(state): State<WebState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut pkg_bytes: Option<Vec<u8>> = None;
    let mut name_override: Option<String> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let fname = field.name().unwrap_or("");
        match fname {
            "file" => {
                if let Ok(bytes) = field.bytes().await {
                    pkg_bytes = Some(bytes.to_vec());
                }
            }
            "name" => {
                if let Ok(text) = field.text().await {
                    if !text.trim().is_empty() {
                        name_override = Some(text.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }

    let bytes = match pkg_bytes {
        Some(b) => b,
        None => return (StatusCode::BAD_REQUEST, "missing file").into_response(),
    };

    // Stage upload to packages directory using content digest
    let digest = sha256_hex(&bytes);
    let packages_dir = crate::p2p::state::agent_data_dir()
        .join("artifacts")
        .join("packages")
        .join(&digest);
    if let Err(e) = tokio::fs::create_dir_all(&packages_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create package dir: {}", e),
        )
            .into_response();
    }

    let zip_path = packages_dir.join("package.zip");
    if !zip_path.exists() {
        if let Err(e) = tokio::fs::write(&zip_path, &bytes).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to stage package: {}", e),
            )
                .into_response();
        }
    }

    // Extract manifest.toml and component.wasm and any static/config/seed-data dirs
    let file = match std::fs::File::open(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("open zip: {}", e),
            )
                .into_response()
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(z) => z,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid zip: {}", e)).into_response(),
    };

    // Read manifest
    let mut manifest_text = String::new();
    match archive.by_name("manifest.toml") {
        Ok(mut mf) => {
            use std::io::Read;
            if let Err(e) = mf.read_to_string(&mut manifest_text) {
                return (StatusCode::BAD_REQUEST, format!("read manifest: {}", e)).into_response();
            }
        }
        Err(_) => return (StatusCode::BAD_REQUEST, "manifest.toml not found").into_response(),
    }

    let pkg_manifest: PackageManifest = match toml::from_str(&manifest_text) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest parse error: {}", e),
            )
                .into_response()
        }
    };

    let comp_name = name_override.unwrap_or_else(|| pkg_manifest.component.name.clone());
    let comp_wasm_path = pkg_manifest.component.wasm.clone();

    // Extract files to packages_dir/digest/
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let outpath = packages_dir.join(f.name());
        if (*f.name()).ends_with('/') {
            let _ = std::fs::create_dir_all(&outpath);
        } else {
            if let Some(parent) = outpath.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut outfile = match std::fs::File::create(&outpath) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if std::io::copy(&mut f, &mut outfile).is_err() {
                continue;
            }
        }
    }

    // Resolve WASM on disk and compute digest; verify against manifest if present
    let wasm_abs = packages_dir.join(&comp_wasm_path);
    if !wasm_abs.exists() {
        return (
            StatusCode::BAD_REQUEST,
            format!("component wasm not found: {}", wasm_abs.display()),
        )
            .into_response();
    }
    let wasm_bytes = match std::fs::read(&wasm_abs) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read wasm: {}", e),
            )
                .into_response()
        }
    };
    let wasm_digest = sha256_hex(&wasm_bytes);
    if let Some(manifest_sha) = pkg_manifest.component.sha256.as_ref() {
        if manifest_sha != &wasm_digest {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "wasm sha256 mismatch: manifest={}, actual={}",
                    manifest_sha, wasm_digest
                ),
            )
                .into_response();
        }
    }

    // Build mounts from package manifest
    let mut resolved_mounts: Vec<common::MountSpec> = Vec::new();
    for m in pkg_manifest.mounts.iter() {
        match m.kind {
            common::MountKind::Static | common::MountKind::Config => {
                if let Some(src) = &m.source {
                    let host = packages_dir.join(src);
                    // RO mount
                    resolved_mounts.push(common::MountSpec {
                        host: host.display().to_string(),
                        guest: m.guest.clone(),
                        ro: true,
                    });
                }
            }
            common::MountKind::Work => {
                let host = crate::p2p::state::agent_data_dir()
                    .join("work")
                    .join("components")
                    .join(&comp_name);
                if let Err(_e) = std::fs::create_dir_all(&host) {}
                resolved_mounts.push(common::MountSpec {
                    host: host.display().to_string(),
                    guest: m.guest.clone(),
                    ro: false,
                });
            }
            common::MountKind::State => {
                // Use named volume or component name
                let vol = m.volume.clone().unwrap_or_else(|| comp_name.clone());
                let host = crate::p2p::state::agent_data_dir()
                    .join("state")
                    .join("components")
                    .join(&vol);
                if let Err(_e) = std::fs::create_dir_all(&host) {}
                // Optional one-time seeding if directory is empty
                if let Some(seed_rel) = &m.seed {
                    let is_empty = std::fs::read_dir(&host)
                        .map(|mut it| it.next().is_none())
                        .unwrap_or(true);
                    if is_empty {
                        let seed_src = packages_dir.join(seed_rel);
                        let _ = copy_dir_recursive(&seed_src, &host);
                    }
                }
                resolved_mounts.push(common::MountSpec {
                    host: host.display().to_string(),
                    guest: m.guest.clone(),
                    ro: false,
                });
            }
        }
    }

    // Create ComponentSpec pointing at cached digest path; include resolved mounts
    let spec = ComponentSpec {
        source: format!("cached:{}", wasm_digest),
        sha256_hex: wasm_digest.clone(),
        replicas: Some(1),
        memory_max_mb: Some(64),
        fuel: None,
        epoch_ms: Some(100),
        mounts: if resolved_mounts.is_empty() {
            None
        } else {
            Some(resolved_mounts)
        },
        ports: None,
        visibility: None,
        target_peer_ids: Vec::new(),
        target_tags: Vec::new(),
        start: true,
    };

    // Also stage the wasm into artifacts cache with the standard name pattern used by supervisor restore
    let artifacts_dir = crate::p2p::state::agent_data_dir().join("artifacts");
    if let Err(e) = std::fs::create_dir_all(&artifacts_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cache dir: {}", e),
        )
            .into_response();
    }
    let wasm_cache_path = artifacts_dir.join(format!("{}-{}.wasm", comp_name, &wasm_digest[..16]));
    if !wasm_cache_path.exists() {
        if let Err(e) = std::fs::write(&wasm_cache_path, &wasm_bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stage wasm: {}", e),
            )
                .into_response();
        }
    }

    // Upsert into supervisor and persist
    let desired_component = DesiredComponent {
        name: comp_name.clone(),
        path: wasm_cache_path.clone(),
        spec: spec.clone(),
    };
    state.supervisor.upsert_component(desired_component).await;
    crate::p2p::state::update_persistent_manifest_with_component(&comp_name, spec);

    // Build a minimal JSON without invoking serde to keep this endpoint simple
    let body = format!("{{\"name\":\"{}\",\"digest\":\"{}\"}}", comp_name, digest);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// Install a .realm package from raw bytes and upsert the component into the supervisor.
/// Returns (component_name, package_digest) on success.
pub async fn install_package_from_bytes(
    state: &WebState,
    bytes: Vec<u8>,
    name_override: Option<String>,
) -> Result<(String, String), String> {
    // Stage upload dir by digest
    let digest = sha256_hex(&bytes);
    let packages_dir = crate::p2p::state::agent_data_dir()
        .join("artifacts")
        .join("packages")
        .join(&digest);
    tokio::fs::create_dir_all(&packages_dir)
        .await
        .map_err(|e| format!("create package dir: {}", e))?;
    let zip_path = packages_dir.join("package.zip");
    if !zip_path.exists() {
        tokio::fs::write(&zip_path, &bytes)
            .await
            .map_err(|e| format!("stage package: {}", e))?;
    }

    // Open zip and read manifest
    let file = std::fs::File::open(&zip_path).map_err(|e| format!("open zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {}", e))?;

    let mut manifest_text = String::new();
    let mut mf = archive
        .by_name("manifest.toml")
        .map_err(|_| "manifest.toml not found".to_string())?;
    mf.read_to_string(&mut manifest_text)
        .map_err(|e| format!("read manifest: {}", e))?;
    drop(mf);
    let pkg_manifest: PackageManifest =
        toml::from_str(&manifest_text).map_err(|e| format!("manifest parse error: {}", e))?;

    let comp_name = name_override.unwrap_or_else(|| pkg_manifest.component.name.clone());
    let comp_wasm_path = pkg_manifest.component.wasm.clone();

    // Extract all files under the digest directory
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let outpath = packages_dir.join(f.name());
        if (*f.name()).ends_with('/') {
            let _ = std::fs::create_dir_all(&outpath);
        } else {
            if let Some(parent) = outpath.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut outfile = match std::fs::File::create(&outpath) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if std::io::copy(&mut f, &mut outfile).is_err() {
                continue;
            }
        }
    }

    // Verify wasm and digest
    let wasm_abs = packages_dir.join(&comp_wasm_path);
    if !wasm_abs.exists() {
        return Err(format!("component wasm not found: {}", wasm_abs.display()));
    }
    let wasm_bytes = std::fs::read(&wasm_abs).map_err(|e| format!("read wasm: {}", e))?;
    let wasm_digest = sha256_hex(&wasm_bytes);
    if let Some(manifest_sha) = pkg_manifest.component.sha256.as_ref() {
        if manifest_sha != &wasm_digest {
            return Err(format!(
                "wasm sha256 mismatch: manifest={}, actual={}",
                manifest_sha, wasm_digest
            ));
        }
    }

    // Resolve mounts
    let mut resolved_mounts: Vec<common::MountSpec> = Vec::new();
    for m in pkg_manifest.mounts.iter() {
        match m.kind {
            common::MountKind::Static | common::MountKind::Config => {
                if let Some(src) = &m.source {
                    let host = packages_dir.join(src);
                    resolved_mounts.push(common::MountSpec {
                        host: host.display().to_string(),
                        guest: m.guest.clone(),
                        ro: true,
                    });
                }
            }
            common::MountKind::Work => {
                let host = crate::p2p::state::agent_data_dir()
                    .join("work")
                    .join("components")
                    .join(&comp_name);
                let _ = std::fs::create_dir_all(&host);
                resolved_mounts.push(common::MountSpec {
                    host: host.display().to_string(),
                    guest: m.guest.clone(),
                    ro: false,
                });
            }
            common::MountKind::State => {
                let vol = m.volume.clone().unwrap_or_else(|| comp_name.clone());
                let host = crate::p2p::state::agent_data_dir()
                    .join("state")
                    .join("components")
                    .join(&vol);
                let _ = std::fs::create_dir_all(&host);
                if let Some(seed_rel) = &m.seed {
                    let is_empty = std::fs::read_dir(&host)
                        .map(|mut it| it.next().is_none())
                        .unwrap_or(true);
                    if is_empty {
                        let seed_src = packages_dir.join(seed_rel);
                        let _ = copy_dir_recursive(&seed_src, &host);
                    }
                }
                resolved_mounts.push(common::MountSpec {
                    host: host.display().to_string(),
                    guest: m.guest.clone(),
                    ro: false,
                });
            }
        }
    }

    // Stage wasm into artifacts cache
    let artifacts_dir = crate::p2p::state::agent_data_dir().join("artifacts");
    let _ = std::fs::create_dir_all(&artifacts_dir);
    let wasm_cache_path = artifacts_dir.join(format!("{}-{}.wasm", comp_name, &wasm_digest[..16]));
    if !wasm_cache_path.exists() {
        std::fs::write(&wasm_cache_path, &wasm_bytes).map_err(|e| format!("stage wasm: {}", e))?;
    }

    // Upsert and persist
    let spec = ComponentSpec {
        source: format!("cached:{}", wasm_digest),
        sha256_hex: wasm_digest.clone(),
        replicas: Some(1),
        memory_max_mb: Some(64),
        fuel: None,
        epoch_ms: Some(100),
        mounts: if resolved_mounts.is_empty() {
            None
        } else {
            Some(resolved_mounts)
        },
        ports: None,
        visibility: None,
        target_peer_ids: Vec::new(),
        target_tags: Vec::new(),
        start: true,
    };
    let desired_component = DesiredComponent {
        name: comp_name.clone(),
        path: wasm_cache_path.clone(),
        spec: spec.clone(),
    };
    state.supervisor.upsert_component(desired_component).await;
    crate::p2p::state::update_persistent_manifest_with_component(&comp_name, spec);
    Ok((comp_name, digest))
}

/// Inspect a .realm package (multipart: file) and return manifest and proposed mount mappings (no install)
pub async fn api_deploy_package_inspect(mut multipart: Multipart) -> impl IntoResponse {
    let mut pkg_bytes: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            if let Ok(bytes) = field.bytes().await {
                pkg_bytes = Some(bytes.to_vec());
            }
        }
    }
    let bytes = match pkg_bytes {
        Some(b) => b,
        None => return (StatusCode::BAD_REQUEST, "missing file").into_response(),
    };
    // Parse manifest from zip in-memory
    let rdr = Cursor::new(bytes);
    let mut archive = match zip::ZipArchive::new(rdr) {
        Ok(z) => z,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid zip: {}", e)).into_response(),
    };
    let mut manifest_text = String::new();
    match archive.by_name("manifest.toml") {
        Ok(mut mf) => {
            if let Err(e) = mf.read_to_string(&mut manifest_text) {
                return (StatusCode::BAD_REQUEST, format!("read manifest: {}", e)).into_response();
            }
        }
        Err(_) => return (StatusCode::BAD_REQUEST, "manifest.toml not found").into_response(),
    }
    let pkg_manifest: PackageManifest = match toml::from_str(&manifest_text) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest parse error: {}", e),
            )
                .into_response()
        }
    };
    // Build file inventory
    let mut files: Vec<serde_json::Value> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(f) = archive.by_index(i) {
            let name = f.name().to_string();
            let is_dir = name.ends_with('/');
            let size = if is_dir { 0 } else { f.size() };
            files.push(json!({ "path": name, "is_dir": is_dir, "size": size }));
        }
    }
    // Construct proposed host mounts (without touching disk)
    let comp_name = pkg_manifest.component.name.clone();
    let mut mounts: Vec<serde_json::Value> = Vec::new();
    for m in pkg_manifest.mounts.iter() {
        let (host, ro) = match m.kind {
            common::MountKind::Static | common::MountKind::Config => {
                let src = m.source.clone().unwrap_or_default();
                (format!("<package>/{src}"), true)
            }
            common::MountKind::Work => (
                format!(
                    "{}/work/components/{}",
                    crate::p2p::state::agent_data_dir().display(),
                    comp_name
                ),
                false,
            ),
            common::MountKind::State => {
                let vol = m.volume.clone().unwrap_or_else(|| comp_name.clone());
                (
                    format!(
                        "{}/state/components/{}",
                        crate::p2p::state::agent_data_dir().display(),
                        vol
                    ),
                    false,
                )
            }
        };
        mounts.push(
            json!({ "kind": format!("{:?}", m.kind), "guest": m.guest, "host": host, "ro": ro }),
        );
    }
    let body = json!({
        "component": { "name": pkg_manifest.component.name, "wasm": pkg_manifest.component.wasm, "sha256": pkg_manifest.component.sha256 },
        "mounts": mounts,
        "files": files,
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let md = entry.metadata()?;
        if md.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if md.is_file() {
            std::fs::create_dir_all(target.parent().unwrap())?;
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

pub async fn api_log_components(State(state): State<WebState>) -> Json<Vec<String>> {
    let map = state.logs.lock().await;
    let mut out: Vec<String> = map.keys().cloned().collect();
    out.sort();
    Json(out)
}
