use crate::storage::ContentStore;

pub async fn storage_ls() -> anyhow::Result<()> {
    let store = ContentStore::open();
    for (digest, entry) in store.list() {
        println!(
            "{}\t{} bytes\t{}\t{}",
            digest,
            entry.size_bytes,
            entry.last_accessed_unix,
            if entry.pinned { "pinned" } else { "" },
        );
    }
    Ok(())
}

pub async fn storage_pin(digest: String, pinned: bool) -> anyhow::Result<()> {
    let store = ContentStore::open();
    let res: Result<(), String> = store.pin(&digest, pinned);
    res.map_err(|e| anyhow::anyhow!(e))?;
    println!("ok");
    Ok(())
}

pub async fn storage_gc(target_total_bytes: u64) -> anyhow::Result<()> {
    let store = ContentStore::open();
    let res: Result<(), String> = store.gc_to_target(target_total_bytes);
    res.map_err(|e| anyhow::anyhow!(e))?;
    println!("ok");
    Ok(())
}
