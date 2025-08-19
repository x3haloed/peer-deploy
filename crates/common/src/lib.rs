use serde::{Deserialize, Serialize};
use std::fmt;
use std::collections::BTreeMap;

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
    SubmitJob(JobSpec),
    QueryJobs { status_filter: Option<String>, limit: usize },
    QueryJobStatus { job_id: String },
    CancelJob { job_id: String },
    QueryJobLogs { job_id: String, tail: usize },
    /// Announce known peer addresses to improve mesh connectivity
    AnnouncePeers { peers: Vec<String> },
    /// Inline push of a small blob into the CAS (intended for small attachments)
    /// Bytes must be base64-encoded; receivers verify digest before storing.
    StoragePut { digest: String, bytes_b64: String },
    /// Chunked push for large blobs. Chunks are base64-encoded; receivers
    /// reassemble by digest and verify before storing.
    StoragePutChunk { digest: String, chunk_index: u32, total_chunks: u32, bytes_b64: String },
    // Phase 5A: Storage discovery announcements
    StorageHave { digest: String, size: u64 },
    // Phase 5B: Minimal P2P artifact transfer
    /// Request blob by digest from peers
    StorageGet { digest: String },
    /// Response with blob bytes base64-encoded. Intended for small artifacts only.
    StorageData { digest: String, bytes_b64: String },
    /// Job acceptance broadcast - node claims job execution
    JobAccepted { job_id: String, assigned_node: String },
    /// Job status update broadcasts  
    JobStarted { job_id: String, assigned_node: String },
    JobCompleted { job_id: String, assigned_node: String, exit_code: i32 },
    JobFailed { job_id: String, assigned_node: String, error: String },
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
    #[serde(default)]
    pub trusted_owner_pub_bs58: Option<String>,
    #[serde(default)]
    pub links: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OwnerKeypair {
    pub public_bs58: String,  // ed25519:BASE58
    #[serde(default)]
    pub private_hex: String,  // hex-encoded ed25519 private key
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
    #[serde(default)]
    pub target_platform: Option<String>, // e.g., "linux/x86_64"; optional for backward compat
    pub target_peer_ids: Vec<String>,
    pub target_tags: Vec<String>,
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
    pub mounts: Option<Vec<MountSpec>>, // preopened directories for ad-hoc push
    pub ports: Option<Vec<ServicePort>>, // declared guest ports
    pub visibility: Option<Visibility>,  // gateway binding policy
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
    pub mounts: Option<Vec<MountSpec>>, // preopened directories
    pub ports: Option<Vec<ServicePort>>, // declared guest ports (Service)
    pub visibility: Option<Visibility>,  // gateway binding policy
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountSpec {
    pub host: String,  // host directory path
    pub guest: String, // guest path inside WASI sandbox (e.g., "/www")
    #[serde(default)]
    pub ro: bool,
}

// ===================== Package Format (Phase 3) =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountKind {
    /// Read-only, from package contents, content-addressed
    Static,
    /// Read-only, from package, intended for initial configuration files
    Config,
    /// Read-write, ephemeral, per-replica working directory
    Work,
    /// Read-write, persistent state across restarts/upgrades
    State,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMountSpec {
    pub kind: MountKind,
    /// Guest path inside WASI sandbox
    pub guest: String,
    /// For kind = static|config: path within the package directory (e.g., "static/", "config/")
    #[serde(default)]
    pub source: Option<String>,
    /// For kind = work: optional size limit in MB
    #[serde(default)]
    pub size_mb: Option<u64>,
    /// For kind = state: optional named volume (defaults to "default")
    #[serde(default)]
    pub volume: Option<String>,
    /// For kind = state: optional path within package used to seed the volume on first install only
    #[serde(default)]
    pub seed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageComponent {
    /// Human-readable component name
    pub name: String,
    /// Path of the main WASM file within the package (e.g., "component.wasm")
    pub wasm: String,
    /// Optional pinned sha256 digest of the WASM file
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub component: PackageComponent,
    /// Mount specifications describing how package assets map into the guest
    #[serde(default)]
    pub mounts: Vec<PackageMountSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    #[serde(default = "default_protocol")] 
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Protocol {
    Tcp,
    Udp,
}

fn default_protocol() -> Protocol { Protocol::Tcp }

// Removed legacy static file routing; HTTP is now handled via WASI HTTP.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Visibility {
    Local,
    Public,
}

pub fn sign_bytes_ed25519(private_hex: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use ed25519_dalek::Signer;
    use ed25519_dalek::SigningKey;
    if private_hex.trim().is_empty() {
        anyhow::bail!("owner private key missing; run `realm init` to generate a key, or ensure `owner.key.json` contains `private_hex`");
    }
    let sk_bytes = hex::decode(private_hex)?;
    if sk_bytes.len() != 32 {
        anyhow::bail!("bad key len: expected 32-byte ed25519 private key; run `realm init` to regenerate");
    }
    let sk_array: [u8; 32] = sk_bytes.as_slice().try_into()
        .map_err(|_| anyhow::anyhow!("Invalid private key length"))?;
    let signing = SigningKey::from_bytes(&sk_array);
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

// ===================== Job Orchestration (Phase 2) =====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: String,
    #[serde(default = "default_job_type")]
    pub job_type: JobType, // one-shot | recurring | service
    #[serde(default)]
    pub schedule: Option<String>, // cron expr for recurring jobs

    pub runtime: JobRuntime,
    pub execution: JobExecution,
    #[serde(default)]
    pub targeting: Option<JobTargeting>,
}

fn default_job_type() -> JobType { JobType::OneShot }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobType {
    OneShot,
    Recurring,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum JobRuntime {
    #[serde(rename = "wasm")]
    Wasm {
        /// File or URL. Examples: file:/abs/path.wasm, https://host/path.wasm
        source: String,
        /// Optional pinned digest for integrity
        #[serde(default)]
        sha256_hex: Option<String>,
        #[serde(default = "default_mem_mb")] 
        memory_mb: u64,
        #[serde(default = "default_fuel")] 
        fuel: u64,
        #[serde(default = "default_epoch_ms")] 
        epoch_ms: u64,
        #[serde(default)]
        mounts: Option<Vec<MountSpec>>, // preopened directories for job runtime
    },
    #[serde(rename = "native")]
    Native {
        /// File or URL to a native host binary to execute.
        /// Examples: file:/abs/path/my-bin, https://host/my-bin
        binary: String,
        /// Optional pinned digest for integrity
        #[serde(default)]
        sha256_hex: Option<String>,
        /// Command-line arguments
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables to set
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    #[serde(rename = "qemu")]
    Qemu {
        /// File or URL to a foreign-arch binary executed via QEMU user-mode
        binary: String,
        /// Optional pinned digest for integrity
        #[serde(default)]
        sha256_hex: Option<String>,
        /// Command-line arguments
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables to set
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// Target platform for emulation, e.g., "linux/amd64", "linux/arm64"
        #[serde(default)]
        target_platform: Option<String>,
        /// Optional explicit qemu user-mode binary path (e.g., "/usr/bin/qemu-x86_64")
        #[serde(default)]
        qemu_binary: Option<String>,
    },
}

fn default_mem_mb() -> u64 { 64 }
fn default_fuel() -> u64 { 5_000_000 }
fn default_epoch_ms() -> u64 { 100 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobExecution {
    /// Optional working directory semantic for non-wasm runtimes; ignored for wasm currently
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Optional timeout in minutes
    #[serde(default)]
    pub timeout_minutes: Option<u64>,
    /// Optional list of artifacts to capture after job completion
    #[serde(default)]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    /// Optional list of blobs to fetch to the host filesystem before execution
    /// Each entry specifies a content-addressed source and a destination path
    /// on the host where the blob should be written.
    #[serde(default)]
    pub pre_stage: Vec<PreStageSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTargeting {
    #[serde(default)]
    pub platform: Option<String>, // linux/x86_64, macos/aarch64, etc
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSpec {
    /// Guest path of the artifact (e.g., "/out/app.wasm")
    pub path: String,
    /// Optional friendly name to use when storing/serving
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreStageSpec {
    /// Content-addressed source, e.g. "cas:<sha256>"
    pub source: String,
    /// Absolute or working-dir-relative destination path on host
    pub dest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobArtifact {
    pub name: String,
    /// Absolute host path where the artifact is stored on the agent
    pub stored_path: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    /// Optional content digest (sha256 hex). Present when staged into CAS.
    #[serde(default)]
    pub sha256_hex: Option<String>,
}

// ===================== Job State & Management =====================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInstance {
    pub id: String,
    pub spec: JobSpec,
    pub status: JobStatus,
    pub submitted_at: u64,  // unix timestamp
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub error_message: Option<String>,
    pub assigned_node: Option<String>,
    pub logs: Vec<JobLogEntry>,
    #[serde(default)]
    pub last_scheduled_at: Option<u64>,
    #[serde(default)]
    pub schedule_next_at: Option<u64>,
    #[serde(default)]
    pub artifacts: Vec<JobArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobLogEntry {
    pub timestamp: u64,
    pub level: String,  // info, warn, error, etc.
    pub message: String,
}

impl JobInstance {
    pub fn new(id: String, spec: JobSpec) -> Self {
        Self {
            id,
            spec,
            status: JobStatus::Pending,
            submitted_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            started_at: None,
            completed_at: None,
            exit_code: None,
            error_message: None,
            assigned_node: None,
            logs: Vec::new(),
            last_scheduled_at: None,
            schedule_next_at: None,
            artifacts: Vec::new(),
        }
    }
    
    pub fn start(&mut self, node_id: String) {
        self.status = JobStatus::Running;
        self.assigned_node = Some(node_id);
        self.started_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }
    
    pub fn complete(&mut self, exit_code: i32) {
        self.status = if exit_code == 0 { JobStatus::Completed } else { JobStatus::Failed };
        self.exit_code = Some(exit_code);
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }
    
    pub fn fail(&mut self, error: String) {
        self.status = JobStatus::Failed;
        self.error_message = Some(error);
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }
    
    pub fn cancel(&mut self) {
        self.status = JobStatus::Cancelled;
        self.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }
    
    pub fn add_log(&mut self, level: String, message: String) {
        self.logs.push(JobLogEntry {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            level,
            message,
        });
    }
}
