use anyhow::Context;

use common::sha256_hex;

#[cfg(unix)]
pub async fn install(binary: Option<String>, system: bool) -> anyhow::Result<()> {
    let binary = binary.unwrap_or_else(|| {
        std::env::current_exe()
            .unwrap_or_else(|_| std::path::PathBuf::from("realm-agent"))
            .display()
            .to_string()
    });
    use std::os::unix::fs::{PermissionsExt, symlink};
    if system {
        // System-wide install
        let data_bin_dir = std::path::Path::new("/usr/local/lib/realm-agent/bin");
        if tokio::fs::create_dir_all(&data_bin_dir).await.is_err() {
            println!("failed to create {}. try: sudo mkdir -p {}", data_bin_dir.display(), data_bin_dir.display());
            return Ok(());
        }
        let bin_bytes = tokio::fs::read(&binary).await?;
        let digest = sha256_hex(&bin_bytes);
        let versioned = data_bin_dir.join(format!("realm-agent-{}", &digest[..16]));
        tokio::fs::write(&versioned, &bin_bytes).await?;
        let _ = tokio::fs::set_permissions(&versioned, std::fs::Permissions::from_mode(0o755)).await;

        // Symlink current -> versioned
        let current_link = data_bin_dir.join("current");
        let _ = tokio::fs::remove_file(&current_link).await;
        let _ = symlink(&versioned, &current_link);

        // Convenience symlink in /usr/local/bin
        let usr_bin_link = std::path::Path::new("/usr/local/bin").join("realm-agent");
        let _ = tokio::fs::remove_file(&usr_bin_link).await;
        let _ = symlink(&current_link, &usr_bin_link);

        let unit_path = std::path::Path::new("/etc/systemd/system").join("realm-agent.service");
        let unit = format!("[Unit]\nDescription=Realm Agent\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=multi-user.target\n", current_link.display());
        if tokio::fs::write(&unit_path, unit).await.is_err() {
            println!("failed to write {}. try: sudo tee {} < <(echo unit) && sudo systemctl daemon-reload", unit_path.display(), unit_path.display());
            return Ok(());
        }
        let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
        let _ = std::process::Command::new("systemctl").args(["enable", "--now", "realm-agent"]).status();
        println!("installed and started system service realm-agent");
        return Ok(());
    }

    // User-mode install (dev)
    let bin_dir = dirs::home_dir().context("home dir")?.join(".local/bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    let data_bin_dir = dirs::data_dir().context("data dir")?.join("realm-agent").join("bin");
    tokio::fs::create_dir_all(&data_bin_dir).await?;
    let bin_bytes = tokio::fs::read(&binary).await?;
    let digest = sha256_hex(&bin_bytes);
    let versioned = data_bin_dir.join(format!("realm-agent-{}", &digest[..16]));
    tokio::fs::write(&versioned, &bin_bytes).await?;
    let _ = tokio::fs::set_permissions(&versioned, std::fs::Permissions::from_mode(0o755)).await;

    // current -> versioned in data dir
    let current_link = data_bin_dir.join("current");
    let _ = tokio::fs::remove_file(&current_link).await;
    let _ = symlink(&versioned, &current_link);

    // ~/.local/bin/realm-agent -> current
    let target_link = bin_dir.join("realm-agent");
    let _ = tokio::fs::remove_file(&target_link).await;
    let _ = symlink(&current_link, &target_link);

    let systemd_dir = dirs::config_dir().context("config dir")?.join("systemd/user");
    tokio::fs::create_dir_all(&systemd_dir).await?;
    let service_path = systemd_dir.join("realm-agent.service");
    let service = format!("[Unit]\nDescription=Realm Agent\n\n[Service]\nExecStart={}\nRestart=always\n\n[Install]\nWantedBy=default.target\n", current_link.display());
    tokio::fs::write(&service_path, service).await?;

    if std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status().is_ok() {
        let _ = std::process::Command::new("systemctl").args(["--user", "enable", "--now", "realm-agent"]).status();
        println!("installed and started systemd user service realm-agent");
        println!("note: user services do not start at boot without a user session. to enable lingering: 'loginctl enable-linger $(whoami)'");
    } else {
        println!("service file written to {}. enable with: systemctl --user enable --now realm-agent", service_path.display());
        println!("note: user services do not start at boot without a user session. to enable lingering: 'loginctl enable-linger $(whoami)'");
    }
    Ok(())
}

#[cfg(unix)]
pub async fn install_cli(_system: bool) -> anyhow::Result<()> {
    let binary = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("realm"))
        .display()
        .to_string();

    // User-mode install (dev). We intentionally do not attempt to set up a service for the CLI.
    let bin_dir = dirs::home_dir().context("home dir")?.join(".local/bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    let data_bin_dir = dirs::data_dir().context("data dir")?.join("realm").join("bin");
    tokio::fs::create_dir_all(&data_bin_dir).await?;
    let bin_bytes = tokio::fs::read(&binary).await?;
    let digest = sha256_hex(&bin_bytes);
    let versioned = data_bin_dir.join(format!("realm-{}", &digest[..16]));
    tokio::fs::write(&versioned, &bin_bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&versioned, std::fs::Permissions::from_mode(0o755)).await;
    }

    // current -> versioned in data dir
    let current_link = data_bin_dir.join("current");
    let _ = tokio::fs::remove_file(&current_link).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&versioned, &current_link);
    }

    // ~/.local/bin/realm -> current
    let target_link = bin_dir.join("realm");
    let _ = tokio::fs::remove_file(&target_link).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&current_link, &target_link);
    }

    // macOS does not use systemd; ignore `system` parameter for CLI.
    println!("installed CLI at {} (symlink {})", versioned.display(), target_link.display());
    Ok(())
}

