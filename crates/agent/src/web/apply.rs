#![allow(dead_code)]
use axum::{extract::Multipart, http::StatusCode, response::IntoResponse};

pub async fn api_apply_multipart(mut multipart: Multipart) -> impl IntoResponse {
    let mut version: u64 = 1;
    let mut toml_text: Option<String> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("");
        match name {
            "file" => {
                toml_text = field.text().await.ok();
            }
            "version" => {
                version = field
                    .text()
                    .await
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
            }
            _ => {}
        }
    }
    let toml_text = match toml_text {
        Some(t) => t,
        None => return (StatusCode::BAD_REQUEST, "missing file").into_response(),
    };
    let upload_path = crate::p2p::state::agent_data_dir().join("upload-manifest.toml");
    if tokio::fs::write(&upload_path, toml_text.as_bytes())
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to stage manifest",
        )
            .into_response();
    }
    match crate::cmd::apply(None, Some(upload_path.display().to_string()), version).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("apply failed: {e}")).into_response(),
    }
}
