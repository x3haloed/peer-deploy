use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::supervisor::Supervisor;
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
    info!(address=%bind_addr, "gateway listening (static /www)");

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
            if let Some(line) = req.lines().next() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 { path = parts[1]; }
            }

            let (status_line, content_type, body): (&str, &str, Vec<u8>) = if path == "/" {
                // Simple index listing components with /www mounts
                let desired = sup.get_desired_snapshot().await;
                let mut html = String::from("<html><body><h1>realm gateway</h1><ul>\n");
                for (name, comp) in desired.into_iter() {
                    if let Some(mnts) = comp.spec.mounts.clone() {
                        if mnts.iter().any(|m| m.guest == "/www") {
                            html.push_str(&format!("<li><a href=\"/{}/\">{}</a></li>\n", name, name));
                        }
                    }
                }
                html.push_str("</ul></body></html>");
                ("HTTP/1.1 200 OK", "text/html; charset=utf-8", html.into_bytes())
            } else {
                // Expect /{component}/... => map to static_dir route if present, else /www mount
                let segs: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
                let comp = segs.get(0).copied().unwrap_or("");
                let rest = segs.get(1).copied().unwrap_or("");

                if comp.is_empty() {
                    ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8", b"not found".to_vec())
                } else if let Some(desired) = sup.get_component(comp).await {
                    // Prefer explicit static route if configured; honor path_prefix
                    let route = desired
                        .spec
                        .routes
                        .as_ref()
                        .and_then(|list| list.iter().find(|r| r.static_dir.is_some()));
                    let (base_host_dir, effective_rest) = if let Some(r) = route {
                        let base = PathBuf::from(r.static_dir.as_ref().unwrap());
                        let pfx = r.path_prefix.trim_start_matches('/');
                        let rest_trimmed = rest.trim_start_matches('/');
                        if pfx.is_empty() || rest_trimmed.starts_with(pfx) {
                            let after = if pfx.is_empty() {
                                rest_trimmed
                            } else {
                                rest_trimmed.trim_start_matches(pfx).trim_start_matches('/')
                            };
                            (Some(base), after.to_string())
                        } else {
                            (None, String::new())
                        }
                    } else {
                        // Fallback to /www mount and full rest
                        let base = desired
                            .spec
                            .mounts
                            .clone()
                            .and_then(|v| v.into_iter().find(|m| m.guest == "/www"))
                            .map(|m| PathBuf::from(m.host));
                        (base, rest.to_string())
                    };
                    if let Some(base) = base_host_dir {
                        let sanitized = sanitize_rest(&effective_rest);
                        let rel = if sanitized.is_empty() { "index.html".to_string() } else { sanitized };
                        let file_path: PathBuf = base.join(rel);
                        match tokio::fs::read(&file_path).await {
                            Ok(bytes) => {
                                let ct = guess_content_type(&file_path);
                                ("HTTP/1.1 200 OK", ct, bytes)
                            }
                            Err(_) => {
                                ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8", b"not found".to_vec())
                            }
                        }
                    } else {
                        ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8", b"no /www mount".to_vec())
                    }
                } else {
                    ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8", b"unknown component".to_vec())
                }
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


