use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    #[serde(default)]
    pub allow_native_execution: bool,
    #[serde(default)]
    pub allow_emulation: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self { allow_native_execution: false, allow_emulation: false }
    }
}

pub fn load_policy() -> ExecutionPolicy {
    // Env overrides take precedence for simple toggles
    let env_native = std::env::var("REALM_ALLOW_NATIVE_EXECUTION").ok();
    let env_emul = std::env::var("REALM_ALLOW_EMULATION").ok();

    let mut policy = read_policy_file().unwrap_or_default();
    if let Some(v) = env_native.as_deref() {
        policy.allow_native_execution = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Some(v) = env_emul.as_deref() {
        policy.allow_emulation = v == "1" || v.eq_ignore_ascii_case("true");
    }
    policy
}

fn read_policy_file() -> Option<ExecutionPolicy> {
    let path = crate::p2p::state::agent_data_dir().join("policy.json");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(p) = serde_json::from_slice::<ExecutionPolicy>(&bytes) {
            return Some(p);
        }
    }
    None
}

pub fn qemu_install_help() -> String {
    match std::env::consts::OS {
        "linux" => "Install qemu-user binaries (e.g., `sudo apt install qemu-user` or `sudo dnf install qemu-user-binfmt`).".to_string(),
        "macos" => "Install QEMU with Homebrew: `brew install qemu` (user-mode binaries like qemu-aarch64, qemu-x86_64).".to_string(),
        "freebsd" => "Install QEMU via pkg: `sudo pkg install qemu-user-static`.".to_string(),
        _ => "Install QEMU user-mode emulator (package name varies by OS).".to_string(),
    }
}

pub fn policy_enable_help() -> String {
    let dir = crate::p2p::state::agent_data_dir();
    let path = dir.join("policy.json");
    format!(
        "To enable, create {} with:\n{{\n  \"allow_native_execution\": true,\n  \"allow_emulation\": true\n}}\nOr set env vars REALM_ALLOW_NATIVE_EXECUTION=1 and/or REALM_ALLOW_EMULATION=1.",
        path.display()
    )
}


