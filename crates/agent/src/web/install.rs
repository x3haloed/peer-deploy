#![allow(dead_code)]
use axum::{extract::Multipart, http::StatusCode, response::IntoResponse};

#[cfg(unix)]
pub async fn api_install_cli() -> impl IntoResponse {
    match crate::cmd::install_cli(false).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("install-cli failed: {e}")).into_response(),
    }
}

#[cfg(not(unix))]
pub async fn api_install_cli() -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, "unsupported platform").into_response()
}

pub async fn api_install_agent(mut multipart: Multipart) -> impl IntoResponse {
    #[cfg(not(unix))]
    {
        return (StatusCode::NOT_IMPLEMENTED, "unsupported platform").into_response();
    }
    #[cfg(unix)]
    {
        let mut bin_path: Option<String> = None;
        let mut system_flag: bool = false;
        let mut bin_bytes: Option<Vec<u8>> = None;
        while let Ok(Some(field)) = multipart.next_field().await {
            let name = field.name().unwrap_or("");
            match name {
                "binary" => {
                    bin_bytes = field.bytes().await.ok().map(|b| b.to_vec());
                }
                "system" => {
                    system_flag = field
                        .text()
                        .await
                        .ok()
                        .map(|s| s == "true" || s == "1")
                        .unwrap_or(false);
                }
                _ => {}
            }
        }
        if let Some(bytes) = bin_bytes {
            let tmp = crate::p2p::state::agent_data_dir().join("upload-agent.bin");
            if tokio::fs::write(&tmp, &bytes).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "failed to stage agent")
                    .into_response();
            }
            bin_path = Some(tmp.display().to_string());
        }
        match crate::cmd::install(bin_path, system_flag).await {
            Ok(_) => (StatusCode::OK, "ok").into_response(),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                format!("install-agent failed: {e}"),
            )
                .into_response(),
        }
    }
}
