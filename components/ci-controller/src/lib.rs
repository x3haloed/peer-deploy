#![cfg_attr(feature = "component", no_main)]

#[cfg(feature = "component")]
mod component_impl {
    #[allow(unused_imports)]
    wit_bindgen::generate!({
        world: "ci",
        path: "wit",
    });
    use bindings as bindings;

    use bindings::exports::wasi::http::incoming_handler::Guest;
    use bindings::wasi::http::types as http;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use serde::Deserialize;

    // Read entire IncomingRequest body as bytes
    fn read_body_bytes(req: &http::IncomingRequest) -> Vec<u8> {
        if let Ok(body) = req.consume() {
            if let Ok(stream) = body.stream() {
                // Read up to 1MB
                let mut out: Vec<u8> = Vec::new();
                loop {
                    let chunk = match stream.blocking_read(64 * 1024) { Ok(v) => v, Err(_) => break };
                    if chunk.is_empty() { break; }
                    out.extend_from_slice(&chunk);
                    if out.len() > 1024 * 1024 { break; }
                }
                let _ = http::IncomingBody::finish(body);
                return out;
            }
        }
        Vec::new()
    }

    fn header_value(req: &http::IncomingRequest, name: &str) -> Option<String> {
        if let Ok(h) = req.headers() {
            let needle = name.to_ascii_lowercase();
            for (k, v) in h.entries() {
                if k.to_ascii_lowercase() == needle {
                    return Some(String::from_utf8_lossy(&v).to_string());
                }
            }
        }
        None
    }

    // Parse simple query string "a=b&c=d" (no percent-decoding)
    fn parse_query(q: &str) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        for pair in q.split('&') {
            if pair.is_empty() { continue; }
            let mut it = pair.splitn(2, '=');
            let k = it.next().unwrap_or("");
            let v = it.next().unwrap_or("");
            if !k.is_empty() { out.insert(k.to_string(), v.to_string()); }
        }
        out
    }

    // Read default platforms from config: JSON array or newline/CSV in /config/platforms.(json|txt)
    fn read_platforms_config() -> Vec<String> {
        if let Ok(bytes) = std::fs::read("/config/platforms.json") {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(arr) = v.as_array() { return arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(); }
            }
        }
        if let Ok(txt) = std::fs::read_to_string("/config/platforms.txt") {
            let mut v = Vec::new();
            for line in txt.split(|c| c=='\n' || c==',') { let s = line.trim(); if !s.is_empty() { v.push(s.to_string()); } }
            if !v.is_empty() { return v; }
        }
        vec!["linux/x86_64".to_string()]
    }

    // Verify HMAC if header present and we have a secret mounted at /config/secret
    fn verify_hmac_sha256(body: &[u8], signature_256: &str) -> bool {
        let secret = std::fs::read_to_string("/config/secret").ok();
        if secret.is_none() { return true; }
        let secret = secret.unwrap();
        let sig = signature_256.strip_prefix("sha256=").unwrap_or(signature_256);
        let expected = match hex::decode(sig) { Ok(b) => b, Err(_) => return false };
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.trim().as_bytes()).unwrap();
        mac.update(body);
        mac.verify_slice(&expected).is_ok()
    }

    // Simple outgoing POST helper to local agent using wasi:http
    fn post_form(url_path: &str, boundary: &str, body_bytes: &[u8]) -> Result<u16, ()> {
        let headers = http::Fields::new();
        let req = http::OutgoingRequest::new(headers);
        req.set_method(&http::Method::Post).map_err(|_| ())?;
        req.set_path_with_query(Some(&format!("{}", url_path))).map_err(|_| ())?;
        req.set_scheme(Some(&http::Scheme::Http)).map_err(|_| ())?;
        req.set_authority(Some("127.0.0.1:8080")).map_err(|_| ())?;
        let h = req.headers();
        let _ = h.append("host", b"127.0.0.1:8080");
        let _ = h.append("content-type", format!("multipart/form-data; boundary={}", boundary).as_bytes());
        let _ = h.append("connection", b"close");
        let body = req.body().ok_or(())?;
        {
            let mut w = body.write().map_err(|_| ())?;
            let _ = w.write(body_bytes);
        }
        let _ = http::OutgoingBody::finish(body, None);
        let future = http::handle(req, None).map_err(|_| ())?;
        let resp = future.get().map_err(|_| ())?;
        let status = resp.status();
        Ok(status)
    }

    // Simple HTTP GET to download a tarball into memory (size-limited)
    fn http_get_to_vec(authority: &str, path_and_query: &str, bearer: Option<&str>, max_bytes: usize) -> Result<Vec<u8>, ()> {
        let headers = http::Fields::new();
        let req = http::OutgoingRequest::new(headers);
        req.set_method(&http::Method::Get).map_err(|_| ())?;
        req.set_path_with_query(Some(path_and_query)).map_err(|_| ())?;
        req.set_scheme(Some(&http::Scheme::Https)).map_err(|_| ())?;
        req.set_authority(Some(authority)).map_err(|_| ())?;
        let h = req.headers();
        let _ = h.append("host", authority.as_bytes());
        if let Some(tok) = bearer { let _ = h.append("authorization", format!("Bearer {}", tok).as_bytes()); }
        let body = req.body().ok_or(())?;
        let _ = http::OutgoingBody::finish(body, None);
        let future = http::handle(req, None).map_err(|_| ())?;
        let resp = future.get().map_err(|_| ())?;
        if resp.status() != 200 { return Err(()); }
        let incoming = resp.consume().map_err(|_| ())?;
        let stream = incoming.stream().map_err(|_| ())?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let chunk = match stream.blocking_read(64 * 1024) { Ok(v) => v, Err(_) => break };
            if chunk.is_empty() { break; }
            if out.len() + chunk.len() > max_bytes { return Err(()); }
            out.extend_from_slice(&chunk);
        }
        let _ = http::IncomingBody::finish(incoming);
        Ok(out)
    }

    // Build multipart form with job_toml and optional assets (workspace, gh_token)
    fn build_multipart(job_toml: &str, workspace: Option<&[u8]>, gh_token: Option<&[u8]>) -> (String, Vec<u8>) {
        let boundary = "--------------------------realmci";
        let mut data = Vec::new();
        data.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        data.extend_from_slice(b"Content-Disposition: form-data; name=\"job_toml\"\r\n\r\n");
        data.extend_from_slice(job_toml.as_bytes());
        data.extend_from_slice(b"\r\n");
        if let Some(bytes) = workspace {
            data.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            data.extend_from_slice(b"Content-Disposition: form-data; name=\"asset\"; filename=\"workspace.tar.gz\"\r\n");
            data.extend_from_slice(b"Content-Type: application/gzip\r\n\r\n");
            data.extend_from_slice(bytes);
            data.extend_from_slice(b"\r\n");
        }
        if let Some(bytes) = gh_token {
            data.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            data.extend_from_slice(b"Content-Disposition: form-data; name=\"asset\"; filename=\"gh_token\"\r\n\r\n");
            data.extend_from_slice(bytes);
            data.extend_from_slice(b"\r\n");
        }
        data.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        (boundary.to_string(), data)
    }

    fn make_job_toml(name: &str, platform: &str, repo_full: &str, tag_name: &str, asset_name: &str) -> String {
        format!(
            "name = \"{name}\"\n\n[runtime]\ntype = \"native\"\nbinary = \"/usr/bin/bash\"\nargs = [\"-c\", \"set -e; \
mkdir -p /tmp/workspace; \
tar -xzf /tmp/assets/workspace.tar.gz -C /tmp/workspace; \
cd /tmp/workspace; \
cargo build --release --bin realm; \
ASSET=/{asset_path}; \
if [ -f /tmp/assets/gh_token ]; then \
  TOKEN=$(cat /tmp/assets/gh_token); \
  if [ -n \"$TOKEN\" ]; then \
    echo 'Uploading asset to GitHub release...'; \
    REL=$(curl -s -H \"Authorization: Bearer $TOKEN\" https://api.github.com/repos/{repo}/releases/tags/{tag}); \
    RID=$(printf '%s' \"$REL\" | grep -m1 '"id":' | sed -E 's/.*\"id\": ([0-9]+).*/\\1/'); \
    if [ -n \"$RID\" ]; then \
      curl -s -X POST -H \"Authorization: Bearer $TOKEN\" -H \"Content-Type: application/octet-stream\" --data-binary @\"$ASSET\" \"https://uploads.github.com/repos/{repo}/releases/$RID/assets?name={asset_name}\" > /dev/null || true; \
    else \
      echo 'Release ID not found for tag'; \
    fi; \
  fi; \
fi\"]\nmemory_mb = 4096\n\n[execution]\nworking_dir = \"/tmp\"\ntimeout_minutes = 45\nartifacts = [ {{ path = \"/tmp/workspace/target/release/realm\", name = \"{asset_name}\" }} ]\n\n[targeting]\nplatform = \"{platform}\"\n",
            name = name,
            platform = platform,
            repo = repo_full,
            tag = tag_name,
            asset_name = asset_name,
            asset_path = "tmp/workspace/target/release/realm"
        )
    }

    fn submit_jobs_for_platforms(base_name: &str, platforms: &[String], workspace_bytes: Option<Vec<u8>>, repo_full: &str, tag_name: &str, gh_token_bytes: Option<Vec<u8>>) -> (u16, usize) {
        let mut ok = 0usize;
        let mut last_status = 500u16;
        for p in platforms {
            let job_name = format!("{}-{}", base_name, p.replace('/', "-"));
            let asset_name = format!("realm-{}", p.replace('/', "-"));
            let job_toml = make_job_toml(&job_name, p, repo_full, tag_name, &asset_name);
            let (boundary, form) = build_multipart(&job_toml, workspace_bytes.as_deref(), gh_token_bytes.as_deref());
            let status = post_form("/api/jobs/submit", &boundary, &form).unwrap_or(500);
            last_status = status;
            if status >= 200 && status < 300 { ok += 1; }
        }
        (last_status, ok)
    }

    // Minimal router: accept POST /hook and submit a simple job.
    struct CiController;

    impl Guest for CiController {
        fn handle(req: http::IncomingRequest, _out: http::ResponseOutparam) {
            let method = match req.method() { Ok(m) => m, Err(_) => http::Method::Get };
            let full = req.path_with_query().ok().and_then(|p| p).unwrap_or_else(|| "/".to_string());
            let (path_only, query_map) = if let Some((p,q)) = full.split_once('?') { (p.to_string(), parse_query(q)) } else { (full.clone(), std::collections::BTreeMap::new()) };

            let headers = http::Fields::new();
            let resp = http::OutgoingResponse::new(headers);
            let body = resp.body().expect("body");
            http::ResponseOutparam::set(_out, Ok(resp));

            // Manual trigger: GET/POST /manual?repo=owner/name&tag=vX.Y.Z&platforms=a,b
            if (method == http::Method::Get || method == http::Method::Post) && (path_only == "/manual" || path_only.starts_with("/manual")) {
                let repo = query_map.get("repo").cloned();
                let tag = query_map.get("tag").cloned();
                let platforms = if let Some(p) = query_map.get("platforms") {
                    p.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>()
                } else { read_platforms_config() };

                let mut downloaded: Option<Vec<u8>> = None;
                if let (Some(r), Some(t)) = (repo.clone(), tag.clone()) {
                    let url_authority = "codeload.github.com";
                    let path = format!("/{}/tar.gz/{}", r, t);
                    downloaded = http_get_to_vec(url_authority, &path, None, 50 * 1024 * 1024).ok();
                }
                let workspace_bytes = downloaded.or_else(|| std::fs::read("/workspace/workspace.tar.gz").ok());
                let base_name = tag.clone().unwrap_or_else(|| "manual".to_string());
                let gh_tok = std::fs::read("/config/github_token").ok();
                let repo_full = repo.unwrap_or_else(|| "".to_string());
                let tag_name = tag.unwrap_or_else(|| base_name.clone());
                let (status, ok_count) = submit_jobs_for_platforms(&format!("build-{}", base_name), &platforms, workspace_bytes, &repo_full, &tag_name, gh_tok);
                let mut w = body.write().expect("write");
                let _ = w.write(format!("submitted {} job(s) (status {})\n", ok_count, status).as_bytes());
                drop(w);
                let _ = http::OutgoingBody::finish(body, Some(http::StatusCode::from(200)));
                return;
            }

            if method == http::Method::Post && (path_only == "/hook" || path_only.starts_with("/hook")) {
                let body_bytes = read_body_bytes(&req);
                // Optional HMAC verification
                if let Some(sig) = header_value(&req, "X-Hub-Signature-256") {
                    if !verify_hmac_sha256(&body_bytes, &sig) {
                        let mut w = body.write().expect("write");
                        let _ = w.write(b"bad signature\n");
                        drop(w);
                        let _ = http::OutgoingBody::finish(body, Some(http::StatusCode::from(401)));
                        return;
                    }
                }

                // Parse event type & tag name
                let event = header_value(&req, "X-GitHub-Event").unwrap_or_default();
                let mut tag_name: Option<String> = None;
                let mut repo_full: Option<String> = None; // owner/repo
                if event == "release" {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                        tag_name = v.get("release").and_then(|r| r.get("tag_name")).and_then(|t| t.as_str()).map(|s| s.to_string())
                            .or_else(|| v.get("tag_name").and_then(|t| t.as_str()).map(|s| s.to_string()));
                        repo_full = v.get("repository").and_then(|r| r.get("full_name")).and_then(|s| s.as_str()).map(|s| s.to_string());
                    }
                } else if event == "push" {
                    // Tag push: ref like "refs/tags/v1.2.3"
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                        if let Some(r) = v.get("ref").and_then(|r| r.as_str()) { if r.starts_with("refs/tags/") { tag_name = Some(r.trim_start_matches("refs/tags/").to_string()); } }
                        repo_full = v.get("repository").and_then(|r| r.get("full_name")).and_then(|s| s.as_str()).map(|s| s.to_string());
                    }
                }

                if tag_name.is_none() {
                    let mut w = body.write().expect("write");
                    let _ = w.write(b"ignored\n");
                    drop(w);
                    let _ = http::OutgoingBody::finish(body, Some(http::StatusCode::from(200)));
                    return;
                }

                // Try to fetch tarball from GitHub if available
                let mut downloaded: Option<Vec<u8>> = None;
                if let (Some(tag), Some(repo)) = (tag_name.clone(), repo_full.clone()) {
                    // unauthenticated API: https://api.github.com/repos/{owner}/{repo}/tarball/{tag}
                    // Prefer codeload which serves raw: https://codeload.github.com/{owner}/{repo}/tar.gz/{tag}
                    let url_authority = "codeload.github.com";
                    let path = format!("/{}/tar.gz/{}", repo, tag);
                    downloaded = http_get_to_vec(url_authority, &path, None, 50 * 1024 * 1024).ok();
                }

                // Prefer downloaded tarball; fallback to preopened workspace file if available
                let workspace_bytes = downloaded.or_else(|| std::fs::read("/workspace/workspace.tar.gz").ok());
                let platforms = read_platforms_config();
                let base = tag_name.clone().unwrap_or_else(|| "release".to_string());
                let gh_tok = std::fs::read("/config/github_token").ok();
                let repo_full = repo_full.unwrap_or_else(|| "".to_string());
                let tag_val = tag_name.clone().unwrap_or_else(|| base.clone());
                let (status, ok_count) = submit_jobs_for_platforms(&format!("build-{}", base), &platforms, workspace_bytes, &repo_full, &tag_val, gh_tok);
                let mut w = body.write().expect("write");
                let _ = w.write(format!("submitted {} job(s) (status {})\n", ok_count, status).as_bytes());
                drop(w);
                let _ = http::OutgoingBody::finish(body, Some(http::StatusCode::from(200)));
                return;
            }

            let mut w = body.write().expect("write");
            let _ = w.write(b"not found\n");
            drop(w);
            let _ = http::OutgoingBody::finish(body, Some(http::StatusCode::from(404)));
        }
    }

    bindings::export!(CiController with_types_in bindings);
}

#[cfg(not(feature = "component"))]
mod non_component_stub {
    // No-op library to keep workspace `cargo build` happy when not building the component target.
}


