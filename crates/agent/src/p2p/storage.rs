use std::time::Duration;

#[derive(Debug)]
pub struct StorageRequest {
    pub digest: String,
    pub resp: tokio::sync::oneshot::Sender<Option<Vec<u8>>>,
}

#[derive(Clone)]
pub struct P2PStorage {
    tx: tokio::sync::mpsc::UnboundedSender<StorageRequest>,
}

impl P2PStorage {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<StorageRequest>) -> Self {
        Self { tx }
    }

    pub async fn get(&self, digest: String, timeout: Duration) -> Option<Vec<u8>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Ignore send errors (receiver might have dropped); behave as not found
        let _ = self.tx.send(StorageRequest { digest, resp: tx });
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(bytes_opt)) => bytes_opt,
            _ => None,
        }
    }
}


