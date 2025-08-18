use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::tokio::TokioIo;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;
use tokio::net::TcpListener;
use tracing::{info, warn, error};

use crate::supervisor::Supervisor;
use crate::runner::invoke_http_component_once;
use crate::p2p::metrics::Metrics;

pub async fn serve_gateway(supervisor: Arc<Supervisor>, metrics: Option<Arc<Metrics>>, bind_addr: &str) {
    let addr: SocketAddr = match bind_addr.parse() {
        Ok(addr) => addr,
        Err(e) => {
            error!(address=%bind_addr, error=%e, "Invalid gateway bind address");
            return;
        }
    };

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!(address=%bind_addr, error=%e, "Gateway bind failed");
            return;
        }
    };

    info!(address=%bind_addr, "Gateway listening (WASI HTTP)");

    loop {
        let (stream, remote_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!(error=%e, "Gateway accept failed");
                continue;
            }
        };

        let supervisor = supervisor.clone();
        let metrics = metrics.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| {
                handle_request(req, supervisor.clone(), metrics.clone(), remote_addr)
            });

            let io = TokioIo::new(stream);
            if let Err(e) = http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                warn!(error=%e, remote=%remote_addr, "HTTP connection error");
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    supervisor: Arc<Supervisor>,
    metrics: Option<Arc<Metrics>>,
    _remote_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let start_time = std::time::Instant::now();
    
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    
    // Extract headers before consuming the request
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    
    // Parse component name from path: /component_name/rest
    let segments: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
    let component_name = segments.get(0).copied().unwrap_or("");
    let rest_path = segments.get(1).copied().unwrap_or("");
    
    let (status, body) = if component_name.is_empty() {
        (StatusCode::NOT_FOUND, "Component name required".into())
    } else if let Some(desired) = supervisor.get_component(component_name).await {
        // Read request body
        let body_bytes = match req.collect().await {
            Ok(collected) => collected.to_bytes().to_vec(),
            Err(e) => {
                warn!(error=%e, "Failed to read request body");
                return Ok(create_response(StatusCode::BAD_REQUEST, "Failed to read request body"));
            }
        };

        // Invoke the component
        match invoke_http_component_once(
            &desired.path.to_string_lossy(),
            component_name,
            &method,
            rest_path,
            headers,
            body_bytes,
        ).await {
            Ok((code, _response_headers, response_body)) => {
                let status_code = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
                (status_code, response_body)
            }
            Err(e) => {
                warn!(component=%component_name, error=%e, "HTTP component invocation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "Component error".into())
            }
        }
    } else {
        (StatusCode::NOT_FOUND, format!("Component '{}' not found", component_name).into())
    };

    // Update metrics
    if let Some(m) = &metrics {
        use std::sync::atomic::Ordering;
        m.gateway_requests_total.fetch_add(1, Ordering::Relaxed);
        m.gateway_last_latency_ms.store(start_time.elapsed().as_millis() as u64, Ordering::Relaxed);
        if !status.is_success() {
            m.gateway_errors_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    Ok(create_response(status, body))
}

fn create_response<T: Into<Bytes>>(status: StatusCode, body: T) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("connection", "close")
        .body(Full::new(body.into()))
        .unwrap()
}