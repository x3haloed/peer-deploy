use tokio::io::{duplex, AsyncBufReadExt, BufReader};
use tracing::{error, info};
use wasmtime::{
    component::{Component, Linker as CLinker, ResourceTable, Val},
    Config, Engine, ResourceLimiter, Store,
};

use crate::p2p::metrics::{push_log, SharedLogs};
use wasmtime_wasi::pipe::AsyncWriteStream;
use wasmtime_wasi::AsyncStdoutStream;

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

pub async fn run_wasm_module_with_limits(
    wasm_path: &str,
    component_name: &str,
    logs: SharedLogs,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
) -> anyhow::Result<()> {
    let wasm = tokio::fs::read(wasm_path).await?;

    let mut cfg = Config::new();
    cfg.wasm_component_model(true)
        .wasm_multi_memory(true)
        .epoch_interruption(true)
        .consume_fuel(true);
    let engine = Engine::new(&cfg)?;

    let (stdout_r, stdout_w) = duplex(1024);
    let (stderr_r, stderr_w) = duplex(1024);
    let wasi = wasmtime_wasi::WasiCtxBuilder::new()
        .stdout(AsyncStdoutStream::new(AsyncWriteStream::new(
            1024, stdout_w,
        )))
        .stderr(AsyncStdoutStream::new(AsyncWriteStream::new(
            1024, stderr_w,
        )))
        .build();

    // readers for stdout/stderr pushing into ring buffers
    {
        let logs = logs.clone();
        let name = component_name.to_string();
        tokio::spawn(async move {
            let mut rdr = BufReader::new(stdout_r);
            let mut line = String::new();
            loop {
                line.clear();
                match rdr.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => push_log(&logs, &name, format!("stdout: {}", line.trim_end())).await,
                    Err(e) => {
                        push_log(&logs, &name, format!("stdout read error: {e}")).await;
                        break;
                    }
                }
            }
        });
    }
    {
        let logs = logs.clone();
        let name = component_name.to_string();
        tokio::spawn(async move {
            let mut rdr = BufReader::new(stderr_r);
            let mut line = String::new();
            loop {
                line.clear();
                match rdr.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => push_log(&logs, &name, format!("stderr: {}", line.trim_end())).await,
                    Err(e) => {
                        push_log(&logs, &name, format!("stderr read error: {e}")).await;
                        break;
                    }
                }
            }
        });
    }

    let limiter = MemoryLimiter {
        max_bytes: (memory_max_mb * 1024 * 1024) as usize,
    };
    let mut store = Store::new(
        &engine,
        StoreData {
            table: ResourceTable::new(),
            wasi,
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
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;
    let instance = linker.instantiate(&mut store, &component)?;
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

    handle.abort();
    Ok(())
}
