use crate::policy::{load_policy, save_policy, ExecutionPolicy};

pub async fn policy_show() -> anyhow::Result<()> {
    let pol = load_policy();
    println!("{}", serde_json::to_string_pretty(&pol)?);
    Ok(())
}

pub async fn policy_set(native: Option<bool>, qemu: Option<bool>) -> anyhow::Result<()> {
    let mut pol = load_policy();
    if let Some(n) = native { pol.allow_native_execution = n; }
    if let Some(q) = qemu { pol.allow_emulation = q; }
    save_policy(&pol).map_err(|e| anyhow::anyhow!(e))?;
    println!("policy saved");
    Ok(())
}


