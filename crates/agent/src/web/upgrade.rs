#![allow(dead_code)]
use axum::{extract::Multipart, http::StatusCode, response::IntoResponse};

pub async fn api_upgrade_multipart(mut multipart: Multipart) -> impl IntoResponse {
    let mut bins: Vec<Vec<u8>> = Vec::new();
    let mut plats: Vec<String> = Vec::new();
    let mut version: u64 = 1;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("");
        match name {
            "file" => {
                if let Ok(bytes) = field.bytes().await {
                    bins.push(bytes.to_vec());
                }
            }
            "version" => {
                version = field
                    .text()
                    .await
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
            }
            "platform" => {
                if let Ok(s) = field.text().await {
                    if !s.trim().is_empty() {
                        plats.push(s);
                    }
                }
            }
            _ => {}
        }
    }
    if bins.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing file").into_response();
    }
    let mut any_err: Option<String> = None;
    for (idx, bin) in bins.into_iter().enumerate() {
        let plat = plats.get(idx).cloned();
        let digest = common::sha256_hex(&bin);
        let upload_path =
            crate::p2p::state::agent_data_dir().join(format!("upload-agent-{}.bin", &digest[..16]));
        if tokio::fs::write(&upload_path, &bin).await.is_err() {
            any_err = Some("failed to stage upload".into());
            break;
        }
        if let Err(e) = crate::cmd::upgrade(
            upload_path.display().to_string(),
            version,
            plat,
            vec![],
            vec![],
        )
        .await
        {
            any_err = Some(format!("upgrade failed: {}", e));
            break;
        }
    }
    match any_err {
        None => (StatusCode::OK, "ok").into_response(),
        Some(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}
