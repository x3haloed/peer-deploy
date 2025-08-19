use axum::{
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
};

pub async fn api_connect_peer(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let addr = body.get("addr").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if addr.is_empty() {
        return (StatusCode::BAD_REQUEST, "addr required").into_response();
    }
    let mut list = crate::cmd::util::read_bootstrap().await.unwrap_or_default();
    if !list.iter().any(|s| s == &addr) {
        list.push(addr.clone());
        if crate::cmd::util::write_bootstrap(&list).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to persist bootstrap").into_response();
        }
    }
    (StatusCode::OK, "ok").into_response()
}


