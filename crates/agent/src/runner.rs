use tokio::io::{duplex, AsyncBufReadExt, BufReader};
use tracing::{error, info};
use wasmtime::{
    component::{Component, Linker as CLinker, ResourceTable, Val},
    Config, Engine, ResourceLimiter, Store,
};

use crate::p2p::metrics::{push_log, Metrics, SharedLogs};
use common::MountSpec;
use wasmtime_wasi::pipe::AsyncWriteStream;
use wasmtime_wasi::AsyncStdoutStream;
use wasmtime_wasi::{DirPerms, FilePerms};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};
use wasmtime_wasi_http::bindings::http::types::Scheme;
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use hyper::body::Body;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;
use core::convert::Infallible;

struct MemoryLimiter {
    max_bytes: usize,
}

impl ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: u32,
        _desired: u32,
        _maximum: Option<u32>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }
}

struct StoreData {
    table: ResourceTable,
    wasi: wasmtime_wasi::WasiCtx,
    http: WasiHttpCtx,
    limiter: MemoryLimiter,
}

impl wasmtime_wasi::WasiView for StoreData {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        &mut self.wasi
    }
}

impl WasiHttpView for StoreData {
    fn ctx(&mut self) -> &mut WasiHttpCtx { &mut self.http }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}

pub async fn run_wasm_module_with_limits(
    wasm_path: &str,
    component_name: &str,
    logs: SharedLogs,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    metrics: Option<std::sync::Arc<Metrics>>,
    mounts: Option<Vec<MountSpec>>,
) -> anyhow::Result<()> {
    let wasm = tokio::fs::read(wasm_path).await?;

    let mut cfg = Config::new();
    cfg.wasm_component_model(true)
        .async_support(true)
        .wasm_multi_memory(true)
        .epoch_interruption(true)
        .consume_fuel(true);
    let engine = Engine::new(&cfg)?;

    let (stdout_r, stdout_w) = duplex(1024);
    let (stderr_r, stderr_w) = duplex(1024);
    let mut builder = wasmtime_wasi::WasiCtxBuilder::new();
    builder.stdout(AsyncStdoutStream::new(AsyncWriteStream::new(1024, stdout_w)));
    builder.stderr(AsyncStdoutStream::new(AsyncWriteStream::new(1024, stderr_w)));

    // Preopen directories as requested in spec (best-effort; logs on failure)
    if let Some(mounts) = mounts {
        for m in mounts.into_iter() {
            let (dperms, fperms) = if m.ro {
                (DirPerms::READ, FilePerms::READ)
            } else {
                (DirPerms::READ | DirPerms::MUTATE, FilePerms::READ | FilePerms::WRITE)
            };
            match builder.preopened_dir(&m.host, m.guest.as_str(), dperms, fperms) {
                Ok(_) => {
                    if m.ro { push_log(&logs, component_name, format!("mounted {} -> {} (ro)", m.host, m.guest)).await; }
                    else { push_log(&logs, component_name, format!("mounted {} -> {}", m.host, m.guest)).await; }
                }
                Err(e) => {
                    push_log(&logs, component_name, format!("mount failed {} -> {}: {}", m.host, m.guest, e)).await;
                }
            }
        }
    }
    let wasi = builder.build();

    // readers for stdout/stderr pushing into ring buffers
    let logs_out = logs.clone();
    let name_out = component_name.to_string();
    let out_task = tokio::spawn(async move {
        let mut rdr = BufReader::new(stdout_r);
        let mut line = String::new();
        loop {
            line.clear();
            match rdr.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    push_log(&logs_out, &name_out, format!("stdout: {}", line.trim_end())).await
                }
                Err(e) => {
                    push_log(&logs_out, &name_out, format!("stdout read error: {e}")).await;
                    break;
                }
            }
        }
    });
    let logs_err = logs.clone();
    let name_err = component_name.to_string();
    let err_task = tokio::spawn(async move {
        let mut rdr = BufReader::new(stderr_r);
        let mut line = String::new();
        loop {
            line.clear();
            match rdr.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    push_log(&logs_err, &name_err, format!("stderr: {}", line.trim_end())).await
                }
                Err(e) => {
                    push_log(&logs_err, &name_err, format!("stderr read error: {e}")).await;
                    break;
                }
            }
        }
    });

    // Track an approximate memory watermark to update metrics less often
    let mut last_mem_bytes: u64 = 0;
    let limiter = MemoryLimiter {
        max_bytes: (memory_max_mb * 1024 * 1024) as usize,
    };
    let mut store = Store::new(
        &engine,
        StoreData {
            table: ResourceTable::new(),
            wasi,
            http: WasiHttpCtx::new(),
            limiter,
        },
    );

    store.limiter(|data| &mut data.limiter);
    if fuel > 0 {
        if let Err(e) = store.set_fuel(fuel) {
            error!(error = %e, "failed to set fuel");
        }
    }
    store.set_epoch_deadline(1);

    let engine2 = engine.clone();
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(epoch_ms));
        loop {
            ticker.tick().await;
            engine2.increment_epoch();
        }
    });

    let component = Component::from_binary(&engine, &wasm)?;
    let mut linker = CLinker::<StoreData>::new(&engine);
    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    let instance = linker.instantiate_async(&mut store, &component).await?;
    info!(path = %wasm_path, "component instantiated with limits");

    // Try to call the command world's entrypoint: 'run'
    let mut invoked = false;
    if let Ok(func) = instance.get_typed_func::<(), ()>(&mut store, "run") {
        func.call(&mut store, ())?;
        invoked = true;
    }
    if !invoked {
        if let Some(func_any) = instance.get_func(&mut store, "run") {
            let mut no_results: [Val; 0] = [];
            if func_any.call(&mut store, &[], &mut no_results).is_ok() {
                invoked = true;
            }
        }
    }
    if invoked {
        info!(path = %wasm_path, "component run() completed");
    } else {
        info!(path = %wasm_path, "component has no 'run' export or signature mismatch");
    }

    // Update memory usage gauge if present
    if let Some(m) = &metrics {
        // Best-effort: wasm memory size not directly exposed for components; approximate by limit
        let approx = (memory_max_mb * 1024 * 1024) as u64;
        if approx != last_mem_bytes {
            m.set_mem_current_bytes(approx);
            last_mem_bytes = approx;
        }
    }

    handle.abort();
    let _ = out_task.await;
    let _ = err_task.await;
    // Record fuel used if available (API does not expose consumed fuel for components in this version)
    let _ = metrics;
    Ok(())
}

/// One-shot HTTP handler invocation placeholder.
/// For now, this returns 501 Not Implemented until WASI HTTP is wired.
pub async fn invoke_http_component_once(
    wasm_path: &str,
    component_name: &str,
    method: &str,
    path: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> anyhow::Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    // Create a hyper Request with a body type compatible with wasi-http new_incoming_request
    let body_full = Full::new(Bytes::from(body)).map_err(|_e: Infallible| match _e {});
    let uri = if path.is_empty() { "http://component/".to_string() } else { format!("http://component/{}", path.trim_start_matches('/')) };
    let mut req = hyper::Request::builder().method(method).uri(uri).body(body_full)?;
    {
        let h = req.headers_mut();
        for (k, v) in headers {
            if let Ok(name) = hyper::header::HeaderName::from_bytes(k.as_bytes()) {
                if let Ok(val) = hyper::header::HeaderValue::from_str(&v) { h.append(name, val); }
            }
        }
    }
    let resp = invoke_http_component_hyper(wasm_path, component_name, req).await?;
    let (parts, body) = resp.into_parts();
    let collected = BodyExt::collect(body).await?;
    let bytes = collected.to_bytes();
    let mut out_headers = Vec::new();
    for (k, v) in parts.headers.iter() { out_headers.push((k.to_string(), v.to_str().unwrap_or("").to_string())); }
    Ok((parts.status.as_u16(), out_headers, bytes.to_vec()))
}

pub async fn invoke_http_component_hyper<B>(
    wasm_path: &str,
    component_name: &str,
    req: hyper::Request<B>,
) -> anyhow::Result<hyper::Response<HyperOutgoingBody>>
where
    B: Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
{
    // Configure Wasmtime engine for components
    let mut cfg = Config::new();
    cfg.wasm_component_model(true).async_support(true);
    let engine = Engine::new(&cfg)?;
    let component = Component::from_file(&engine, wasm_path)?;
    let mut linker = CLinker::<HttpStore>::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;
    wasmtime_wasi_http::add_to_linker_sync(&mut linker)?;
    let pre = ProxyPre::new(linker.instantiate_pre(&component)?)?;

    let mut store = wasmtime::Store::new(&engine, HttpStore::new(component_name));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let incoming = store.data_mut().new_incoming_request(Scheme::Http, req)?;
    let out = store.data_mut().new_response_outparam(sender)?;

    let instance = pre.instantiate_async(&mut store).await?;
    instance
        .wasi_http_incoming_handler()
        .call_handle(&mut store, incoming, out)
        .await?;
    match receiver.await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

struct HttpStore {
    table: ResourceTable,
    wasi: wasmtime_wasi::WasiCtx,
    http: WasiHttpCtx,
    _name: String,
}

impl HttpStore {
    fn new(name: &str) -> Self {
        let wasi = wasmtime_wasi::WasiCtxBuilder::new().build();
        Self { table: ResourceTable::new(), wasi, http: WasiHttpCtx::new(), _name: name.to_string() }
    }
}

impl wasmtime_wasi::WasiView for HttpStore {
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx { &mut self.wasi }
}

impl WasiHttpView for HttpStore {
    fn ctx(&mut self) -> &mut WasiHttpCtx { &mut self.http }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}
