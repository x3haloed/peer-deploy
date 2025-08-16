use serde::{Deserialize, Serialize};
use std::fmt;

pub const REALM_CMD_TOPIC: &str = "realm/cmd/v1";
pub const REALM_STATUS_TOPIC: &str = "realm/status/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Hello { from: String },
    Run { wasm_path: String, memory_max_mb: u64, fuel: u64, epoch_ms: u64 },
    StatusQuery,
    MetricsQuery,
    LogsQuery { component: Option<String>, tail: u64 },
    ApplyManifest(SignedManifest),
    UpgradeAgent(AgentUpgrade),
    PushComponent(PushPackage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub node_id: String,
    pub msg: String,
    pub agent_version: u64,
    pub components_desired: u64,
    pub components_running: u64,
    pub cpu_percent: u64,
    pub mem_percent: u64,
    pub tags: Vec<String>,
    pub drift: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OwnerKeypair {
    pub public_bs58: String,  // ed25519:BASE58
    #[serde(skip_serializing, default)]
    pub private_hex: String,  // never serialize; allow default when missing
}

impl fmt::Debug for OwnerKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerKeypair")
            .field("public_bs58", &self.public_bs58)
            .field("private_hex", &"<redacted>")
            .finish()
    }
}

impl OwnerKeypair {
    pub fn generate() -> anyhow::Result<Self> {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;
        let signing = SigningKey::generate(&mut OsRng);
        let verify = signing.verifying_key();
        let public_bs58 = format!("ed25519:{}", bs58::encode(verify.to_bytes()).into_string());
        let private_hex = hex::encode(signing.to_bytes());
        Ok(Self { public_bs58, private_hex })
    }

    pub fn from_private_hex(hex_str: &str) -> anyhow::Result<Self> {
        use ed25519_dalek::SigningKey;
        let sk_bytes = hex::decode(hex_str)?;
        let signing = SigningKey::from_bytes(sk_bytes.as_slice().try_into().map_err(|_| anyhow::anyhow!("bad key len"))?);
        let verify = signing.verifying_key();
        let public_bs58 = format!("ed25519:{}", bs58::encode(verify.to_bytes()).into_string());
        Ok(Self { public_bs58, private_hex: hex_str.to_string() })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    pub alg: String,                // "ed25519"
    pub owner_pub_bs58: String,     // "ed25519:BASE58..."
    pub version: u64,               // monotonic
    pub manifest_toml: String,      // raw TOML bytes as UTF-8
    pub signature_b64: String,      // base64 signature over manifest bytes
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUpgrade {
    pub alg: String,            // "ed25519"
    pub owner_pub_bs58: String, // "ed25519:BASE58..."
    pub version: u64,           // monotonic
    pub binary_sha256_hex: String,
    pub binary_b64: String,     // base64 of agent binary
    pub signature_b64: String,  // base64 signature over raw binary bytes
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushUnsigned {
    pub alg: String,                // "ed25519"
    pub owner_pub_bs58: String,     // "ed25519:BASE58..."
    pub component_name: String,
    pub target_peer_ids: Vec<String>,
    pub target_tags: Vec<String>,
    pub memory_max_mb: Option<u64>,
    pub fuel: Option<u64>,
    pub epoch_ms: Option<u64>,
    pub replicas: u32,
    pub start: bool,                // start immediately
    pub binary_sha256_hex: String,  // digest of binary_b64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPackage {
    pub unsigned: PushUnsigned,
    pub binary_b64: String,         // base64 of wasm component
    pub signature_b64: String,      // signature over serde_json(unsigned)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub components: std::collections::BTreeMap<String, ComponentSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentSpec {
    pub source: String,     // e.g., file:/path or http(s):/...
    pub sha256_hex: String, // pinned digest
    pub memory_max_mb: Option<u64>,
    pub fuel: Option<u64>,
    pub epoch_ms: Option<u64>,
    pub replicas: Option<u32>,
}

pub fn sign_bytes_ed25519(private_hex: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use ed25519_dalek::Signer;
    use ed25519_dalek::SigningKey;
    let sk_bytes = hex::decode(private_hex)?;
    let signing = SigningKey::from_bytes(sk_bytes.as_slice().try_into().map_err(|_| anyhow::anyhow!("bad key len"))?);
    let sig = signing.sign(data);
    Ok(sig.to_bytes().to_vec())
}

pub fn verify_bytes_ed25519(public_bs58: &str, data: &[u8], signature: &[u8]) -> anyhow::Result<bool> {
    use ed25519_dalek::{Verifier, VerifyingKey, Signature};
    let without_prefix = public_bs58.strip_prefix("ed25519:").unwrap_or(public_bs58);
    let pk_bytes = bs58::decode(without_prefix).into_vec()?;
    let vk = VerifyingKey::from_bytes(pk_bytes.as_slice().try_into().map_err(|_| anyhow::anyhow!("bad pub len"))?)?;
    let sig = Signature::from_bytes(signature.try_into().map_err(|_| anyhow::anyhow!("bad sig len"))?);
    Ok(vk.verify(data, &sig).is_ok())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    hex::encode(out)
}

pub fn serialize_message<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("serialize_message")
}

pub fn deserialize_message<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteUnsigned {
    pub alg: String, // "ed25519"
    pub owner_pub_bs58: String,
    pub bootstrap_multiaddrs: Vec<String>,
    pub realm_id: Option<String>,
    pub exp_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteToken {
    pub unsigned: InviteUnsigned,
    pub signature_b64: String,
}
