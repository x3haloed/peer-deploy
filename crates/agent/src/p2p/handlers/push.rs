use std::{fmt, sync::Arc};

use base64::Engine;
use tracing::info;

use crate::{
    p2p::{
        metrics::push_log,
        state::{
            agent_data_dir, load_trusted_owner, save_trusted_owner,
            update_persistent_manifest_with_component,
        },
    },
    supervisor::DesiredComponent,
};
use common::{
    sha256_hex, verify_bytes_ed25519, ComponentSpec, MountSpec, PushPackage, ServicePort,
};

use super::super::metrics::SharedLogs;

#[derive(Debug)]
pub enum PushAcceptanceError {
    InvalidSignature,
    OwnerMismatch { expected: String, found: String },
    DigestMismatch,
    Decode(base64::DecodeError),
    Verify(anyhow::Error),
    Io(std::io::Error),
}

impl fmt::Display for PushAcceptanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushAcceptanceError::InvalidSignature => write!(f, "invalid signature"),
            PushAcceptanceError::OwnerMismatch { expected, found } => {
                write!(
                    f,
                    "owner mismatch (trusted={}, provided={})",
                    expected, found
                )
            }
            PushAcceptanceError::DigestMismatch => write!(f, "binary digest mismatch"),
            PushAcceptanceError::Decode(e) => write!(f, "invalid base64: {}", e),
            PushAcceptanceError::Verify(e) => write!(f, "signature verification failed: {}", e),
            PushAcceptanceError::Io(e) => write!(f, "filesystem error: {}", e),
        }
    }
}

impl std::error::Error for PushAcceptanceError {}

impl From<base64::DecodeError> for PushAcceptanceError {
    fn from(value: base64::DecodeError) -> Self {
        PushAcceptanceError::Decode(value)
    }
}

impl From<std::io::Error> for PushAcceptanceError {
    fn from(value: std::io::Error) -> Self {
        PushAcceptanceError::Io(value)
    }
}

/// Accept a push package by verifying ownership, staging the artifact, and
/// scheduling it via the supervisor when `start` is enabled.
pub async fn handle_push_package(
    pkg: PushPackage,
    logs: SharedLogs,
    supervisor: Arc<crate::supervisor::Supervisor>,
) -> Result<(), PushAcceptanceError> {
    let component = &pkg.unsigned.component_name;

    // Verify signature over the unsigned payload
    let unsigned_bytes =
        serde_json::to_vec(&pkg.unsigned).expect("PushUnsigned serialization should not fail");
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(&pkg.signature_b64)?;
    let sig_valid = verify_bytes_ed25519(&pkg.unsigned.owner_pub_bs58, &unsigned_bytes, &sig_bytes)
        .map_err(PushAcceptanceError::Verify)?;
    if !sig_valid {
        push_log(&logs, component, "push rejected: invalid signature").await;
        return Err(PushAcceptanceError::InvalidSignature);
    }

    // Verify binary digest matches declaration
    let bin_bytes = base64::engine::general_purpose::STANDARD.decode(&pkg.binary_b64)?;
    let digest = sha256_hex(&bin_bytes);
    if digest != pkg.unsigned.binary_sha256_hex {
        push_log(&logs, component, "push rejected: digest mismatch").await;
        return Err(PushAcceptanceError::DigestMismatch);
    }

    // Enforce trusted owner TOFU policy
    if let Some(trusted) = load_trusted_owner() {
        if trusted != pkg.unsigned.owner_pub_bs58 {
            push_log(
                &logs,
                component,
                format!(
                    "push rejected: owner mismatch (trusted={}, provided={})",
                    trusted, pkg.unsigned.owner_pub_bs58
                ),
            )
            .await;
            return Err(PushAcceptanceError::OwnerMismatch {
                expected: trusted,
                found: pkg.unsigned.owner_pub_bs58.clone(),
            });
        }
    } else {
        save_trusted_owner(&pkg.unsigned.owner_pub_bs58);
        info!(owner=%pkg.unsigned.owner_pub_bs58, "TOFU: trusted owner recorded");
    }

    // Stage artifact in local cache
    let stage_dir = agent_data_dir().join("artifacts");
    tokio::fs::create_dir_all(&stage_dir).await?;
    let file_path = stage_dir.join(format!("{}-{}.wasm", component, &digest[..16]));
    if !file_path.exists() {
        tokio::fs::write(&file_path, &bin_bytes).await?;
    }
    push_log(
        &logs,
        component,
        format!(
            "pushed {} bytes (sha256={})",
            bin_bytes.len(),
            &digest[..16]
        ),
    )
    .await;

    if pkg.unsigned.start {
        let spec = ComponentSpec {
            source: format!("cached:{}", pkg.unsigned.binary_sha256_hex.clone()),
            sha256_hex: pkg.unsigned.binary_sha256_hex.clone(),
            memory_max_mb: pkg.unsigned.memory_max_mb,
            fuel: pkg.unsigned.fuel,
            epoch_ms: pkg.unsigned.epoch_ms,
            replicas: Some(pkg.unsigned.replicas),
            mounts: clone_mounts(&pkg.unsigned.mounts),
            ports: clone_ports(&pkg.unsigned.ports),
            visibility: pkg.unsigned.visibility.clone(),
            target_peer_ids: pkg.unsigned.target_peer_ids.clone(),
            target_tags: pkg.unsigned.target_tags.clone(),
            start: pkg.unsigned.start,
        };
        let desired = DesiredComponent {
            name: component.clone(),
            path: file_path.clone(),
            spec: spec.clone(),
        };
        supervisor.upsert_component(desired).await;
        update_persistent_manifest_with_component(component, spec);
        push_log(&logs, component, "scheduled (upsert)").await;
    } else {
        push_log(&logs, component, "staged (start=false)").await;
    }

    Ok(())
}

fn clone_mounts(mounts: &Option<Vec<MountSpec>>) -> Option<Vec<MountSpec>> {
    mounts.as_ref().map(|list| list.iter().cloned().collect())
}

fn clone_ports(ports: &Option<Vec<ServicePort>>) -> Option<Vec<ServicePort>> {
    ports.as_ref().map(|list| list.iter().cloned().collect())
}

impl From<anyhow::Error> for PushAcceptanceError {
    fn from(value: anyhow::Error) -> Self {
        PushAcceptanceError::Verify(value)
    }
}
