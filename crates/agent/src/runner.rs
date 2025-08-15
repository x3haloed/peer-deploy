use tracing::info;
use wasmtime::{component::{Component, Linker, ResourceTable}, Config, Engine, ResourceLimiter, Store};

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
	_fuel: u64,
	epoch_ms: u64,
) -> anyhow::Result<()> {
	let wasm = tokio::fs::read(wasm_path).await?;

	let mut cfg = Config::new();
	cfg.wasm_component_model(true)
		.wasm_multi_memory(true)
		.epoch_interruption(true);
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

	let engine2 = engine.clone();
	let handle = tokio::spawn(async move {
		let mut ticker = tokio::time::interval(std::time::Duration::from_millis(epoch_ms));
		loop {
			ticker.tick().await;
			engine2.increment_epoch();
		}
	});

	let component = Component::from_binary(&engine, &wasm)?;
	let mut linker = Linker::<StoreData>::new(&engine);
	wasmtime_wasi::add_to_linker_sync(&mut linker)?;

	let _instance = linker.instantiate(&mut store, &component)?;

	info!(path = %wasm_path, "component instantiated with limits");

	handle.abort();
	Ok(())
}
