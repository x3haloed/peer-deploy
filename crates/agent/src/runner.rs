use tracing::{error, info};
use wasmtime::{component::{Component, Linker as CLinker, ResourceTable}, Config, Engine, ResourceLimiter, Store};

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
	fn table(&mut self) -> &mut ResourceTable { &mut self.table }
	fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx { &mut self.wasi }
}

pub async fn run_wasm_module_with_limits(
	wasm_path: &str,
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

	let wasi = wasmtime_wasi::WasiCtxBuilder::new()
		.inherit_stdio()
		.build();

	let limiter = MemoryLimiter { max_bytes: (memory_max_mb * 1024 * 1024) as usize };
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
		if let Err(e) = store.set_fuel(fuel as u64) {
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
		if let Ok(func2) = instance.get_typed_func::<(), Result<(), ()>>(&mut store, "run") {
			let _ = func2.call(&mut store, ());
			invoked = true;
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
