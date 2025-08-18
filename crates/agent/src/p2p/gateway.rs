use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::supervisor::Supervisor;
use crate::runner::invoke_http_component_once;
use crate::p2p::metrics::Metrics;

fn guess_content_type(path: &Path) -> &'static str {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()) {
        match ext.as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" => "application/javascript; charset=utf-8",
            "json" => "application/json; charset=utf-8",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "wasm" => "application/wasm",
            _ => "application/octet-stream",
        }
    } else {
        "application/octet-stream"
    }
}

fn sanitize_rest(rest: &str) -> String {
    let mut out = String::new();
    for segment in rest.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." { continue; }
        if !out.is_empty() { out.push('/'); }
        out.push_str(segment);
    }
    out
}

pub async fn serve_gateway(supervisor: Arc<Supervisor>, metrics: Option<Arc<Metrics>>, bind_addr: &str) {
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!(address=%bind_addr, error=%e, "gateway bind failed");
            return;
        }
    };
    info!(address=%bind_addr, "gateway listening (WASI HTTP)");

    loop {
        let (mut stream, _addr) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => { warn!(error=%e, "gateway accept"); continue; }
        };
        let sup = supervisor.clone();
        let m = metrics.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            let start_time = std::time::Instant::now();
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
            ).await;
            let req = String::from_utf8_lossy(&buf);
            let mut path = "/";
            let mut method = "GET";
            if let Some(line) = req.lines().next() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 { method = parts[0]; path = parts[1]; }
            }

            // Dispatch to component via WASI HTTP incoming-handler
            let segs: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
            let comp = segs.get(0).copied().unwrap_or("");
            let rest = segs.get(1).copied().unwrap_or("");
            let (status_line, content_type, body): (&str, String, Vec<u8>) = if comp.is_empty() {
                ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8".to_string(), b"not found".to_vec())
            } else if let Some(desired) = sup.get_component(comp).await {
                match invoke_http_component_once(&desired.path.to_string_lossy(), comp, method, rest, vec![], vec![]).await {
                    Ok((code, headers, body)) => {
                        let status_line = match code {
                            200 => "HTTP/1.1 200 OK",
                            404 => "HTTP/1.1 404 Not Found",
                            500 => "HTTP/1.1 500 Internal Server Error",
                            501 => "HTTP/1.1 501 Not Implemented",
                            403 => "HTTP/1.1 403 Forbidden",
                            400 => "HTTP/1.1 400 Bad Request",
                            _ => "HTTP/1.1 200 OK",
                        };
                        let mut ct = "application/octet-stream".to_string();
                        for (k, v) in headers.into_iter() {
                            if k.eq_ignore_ascii_case("content-type") { ct = v; break; }
                        }
                        (status_line, ct, body)
                    }
                    Err(_) => {
                        ("HTTP/1.1 500 Internal Server Error", "text/plain; charset=utf-8".to_string(), b"error".to_vec())
                    }
                }
            } else {
                ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8".to_string(), b"unknown component".to_vec())
            };

            let header = format!(
                "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, header.as_bytes()).await;
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, &body).await;
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
            if let Some(mm) = &m {
                use std::sync::atomic::Ordering;
                mm.gateway_requests_total.fetch_add(1, Ordering::Relaxed);
                mm.gateway_last_latency_ms.store(start_time.elapsed().as_millis() as u64, Ordering::Relaxed);
                if !status_line.contains("200") {
                    mm.gateway_errors_total.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }
}


