use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// File-based content-addressable storage with a JSON index for metadata.
/// Layout:
///   {data_dir}/artifacts/blobs/sha256/aa/bb/{full_sha256}
///   {data_dir}/artifacts/index.json
#[derive(Clone)]
pub struct ContentStore {
    base_dir: PathBuf,
    index_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexEntry {
    pub size_bytes: u64,
    pub last_accessed_unix: u64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexFile {
    pub entries: BTreeMap<String, IndexEntry>,
}

impl ContentStore {
    pub fn open() -> Self {
        let data = crate::p2p::state::agent_data_dir();
        let base_dir = data.join("artifacts").join("blobs").join("sha256");
        let _ = std::fs::create_dir_all(&base_dir);
        let index_path = data.join("artifacts").join("index.json");
        Self { base_dir, index_path }
    }

    fn path_for_digest(&self, digest: &str) -> PathBuf {
        let a = &digest[0..2];
        let b = &digest[2..4];
        self.base_dir.join(a).join(b).join(digest)
    }

    fn now_unix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn load_index(&self) -> IndexFile {
        if self.index_path.exists() {
            if let Ok(bytes) = std::fs::read(&self.index_path) {
                if let Ok(idx) = serde_json::from_slice::<IndexFile>(&bytes) {
                    return idx;
                }
            }
        }
        IndexFile::default()
    }

    fn save_index(&self, idx: &IndexFile) -> Result<(), String> {
        let parent = self.index_path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(idx).map_err(|e| e.to_string())?;
        std::fs::write(&self.index_path, bytes).map_err(|e| e.to_string())
    }

    pub fn has(&self, digest: &str) -> bool {
        self.path_for_digest(digest).exists()
    }

    pub fn put_bytes(&self, bytes: &[u8]) -> Result<String, String> {
        let digest = common::sha256_hex(bytes);
        let path = self.path_for_digest(&digest);
        if !path.exists() {
            if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
        // Update index
        let mut idx = self.load_index();
        let size = bytes.len() as u64;
        let entry = idx.entries.entry(digest.clone()).or_default();
        entry.size_bytes = size;
        entry.last_accessed_unix = Self::now_unix();
        self.save_index(&idx)?;
        Ok(digest)
    }

    pub fn get_path(&self, digest: &str) -> Option<PathBuf> {
        let p = self.path_for_digest(digest);
        if p.exists() {
            let mut idx = self.load_index();
            if let Some(entry) = idx.entries.get_mut(digest) {
                entry.last_accessed_unix = Self::now_unix();
                let _ = self.save_index(&idx);
            }
            Some(p)
        } else {
            None
        }
    }

    pub fn list(&self) -> Vec<(String, IndexEntry)> {
        let idx = self.load_index();
        idx.entries.into_iter().collect()
    }

    pub fn pin(&self, digest: &str, value: bool) -> Result<(), String> {
        let mut idx = self.load_index();
        if let Some(entry) = idx.entries.get_mut(digest) {
            entry.pinned = value;
            self.save_index(&idx)
        } else {
            Err("digest not found".into())
        }
    }

    pub fn total_size_bytes(&self) -> u64 {
        let idx = self.load_index();
        idx.entries.values().map(|e| e.size_bytes).sum()
    }

    /// Garbage collect until total size <= target_total_bytes. Never deletes pinned.
    pub fn gc_to_target(&self, target_total_bytes: u64) -> Result<(), String> {
        let mut idx = self.load_index();
        let mut items: Vec<(String, IndexEntry)> = idx.entries.iter().map(|(d, e)| (d.clone(), e.clone())).collect();
        // Sort by last_accessed ascending (LRU)
        items.sort_by_key(|(_, e)| e.last_accessed_unix);
        let mut current: u64 = items.iter().map(|(_, e)| e.size_bytes).sum();
        for (digest, entry) in items {
            if current <= target_total_bytes { break; }
            if entry.pinned { continue; }
            let path = self.path_for_digest(&digest);
            let _ = std::fs::remove_file(&path);
            current = current.saturating_sub(entry.size_bytes);
            idx.entries.remove(&digest);
        }
        self.save_index(&idx)
    }
}


