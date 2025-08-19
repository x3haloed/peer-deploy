use anyhow::Context;

use common::{PackageComponent, PackageManifest, PackageMountSpec, MountKind};

/// Create a .realm package (zip) from a directory containing component.wasm and optional assets
/// Layout expected:
///   <dir>/component.wasm (required)
///   <dir>/static/         (optional)
///   <dir>/config/         (optional)
///   <dir>/seed-data/      (optional)
///
/// The output is a single .realm (zip) file containing manifest.toml, component.wasm,
/// and any present asset directories.
pub async fn package_create(dir: String, name_override: Option<String>, output: Option<String>) -> anyhow::Result<()> {
    let dir_path = std::path::Path::new(&dir);
    if !dir_path.exists() || !dir_path.is_dir() {
        anyhow::bail!("package create: input path must be a directory");
    }

    // Determine component name
    let comp_name = name_override.unwrap_or_else(|| dir_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "component".to_string()));

    // Locate wasm
    let wasm_path = dir_path.join("component.wasm");
    if !wasm_path.exists() {
        anyhow::bail!(format!("component.wasm not found in {}", dir_path.display()));
    }
    let wasm_bytes = tokio::fs::read(&wasm_path).await.context("read component.wasm")?;
    let wasm_sha256 = common::sha256_hex(&wasm_bytes);

    // Build manifest
    let mut mounts: Vec<PackageMountSpec> = Vec::new();
    if dir_path.join("static").exists() {
        mounts.push(PackageMountSpec { kind: MountKind::Static, guest: "/www".to_string(), source: Some("static/".to_string()), size_mb: None, volume: None, seed: None });
    }
    if dir_path.join("config").exists() {
        mounts.push(PackageMountSpec { kind: MountKind::Config, guest: "/etc/app".to_string(), source: Some("config/".to_string()), size_mb: None, volume: None, seed: None });
    }
    if dir_path.join("seed-data").exists() {
        mounts.push(PackageMountSpec { kind: MountKind::State, guest: "/data".to_string(), source: None, size_mb: None, volume: Some(comp_name.clone()), seed: Some("seed-data/".to_string()) });
    }

    let manifest = PackageManifest {
        component: PackageComponent { name: comp_name.clone(), wasm: "component.wasm".to_string(), sha256: Some(wasm_sha256) },
        mounts,
    };

    let manifest_toml = toml::to_string_pretty(&manifest).context("serialize manifest")?;

    // Determine output path
    let out_path = if let Some(out) = output { std::path::PathBuf::from(out) } else {
        let base = dir_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| comp_name.clone());
        let filename = if base.ends_with(".realm") { base } else { format!("{}.realm", base) };
        dir_path.parent().unwrap_or_else(|| std::path::Path::new(".")).join(filename)
    };

    // Write zip (.realm)
    let file = std::fs::File::create(&out_path).context("create output")?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // manifest.toml
    zip.start_file("manifest.toml", options).context("zip start manifest")?;
    use std::io::Write;
    zip.write_all(manifest_toml.as_bytes()).context("zip write manifest")?;

    // component.wasm
    zip.start_file("component.wasm", options).context("zip start wasm")?;
    zip.write_all(&wasm_bytes).context("zip write wasm")?;

    // Add directories if present
    for name in ["static", "config", "seed-data"].iter() {
        let p = dir_path.join(name);
        if p.exists() && p.is_dir() {
            add_dir_to_zip(&mut zip, &p, &std::path::PathBuf::from(name))?;
        }
    }

    zip.finish().context("zip finish")?;

    println!("Created package: {}", out_path.display());
    Ok(())
}

fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(zip: &mut zip::ZipWriter<W>, src: &std::path::Path, rel: &std::path::Path) -> anyhow::Result<()> {
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // Ensure directory entry exists (optional for unzip tools)
    let rel_s = format!("{}/", rel.display());
    zip.add_directory(rel_s, options).ok();

    for entry in std::fs::read_dir(src).context("read_dir")? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let rel_child = rel.join(name);
        if path.is_dir() {
            add_dir_to_zip(zip, &path, &rel_child)?;
        } else if path.is_file() {
            let bytes = std::fs::read(&path).context("read file")?;
            let rel_str = rel_child.to_string_lossy().to_string();
            zip.start_file(rel_str, options).context("zip start file")?;
            use std::io::Write;
            zip.write_all(&bytes).context("zip write file")?;
        }
    }
    Ok(())
}


