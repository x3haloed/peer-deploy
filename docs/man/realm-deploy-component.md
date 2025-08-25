## realm deploy-component

Build a cargo-component and push to agents.

### Name

realm deploy-component - build a WASI component with cargo and deploy in one step

### Synopsis

```
realm deploy-component [--path <DIR>] [--package <NAME>] [--profile <debug|release>] [--features <LIST>] [--peer <PEER_ID> ...] [--tag <TAG> ...] [--name <NAME>] [--start <true|false>]
```

### Options

- `--path <DIR>`: Path to the cargo project directory (containing `Cargo.toml`). Default: `.`
- `--package <NAME>`: Cargo package name.
- `--profile <debug|release>`: Build profile. Default: `release`.
- `--features <LIST>`: Additional cargo features (comma-separated). Default: `component`.
- `--peer <PEER_ID>`: Target peers by PeerId. Repeatable.
- `--tag <TAG>`: Target peers by tag/role. Repeatable.
- `--name <NAME>`: Component name for deployment (defaults to package name).
- `--start <true|false>`: Start immediately after push. Default: `true`.

### Files

- Reads the signing owner key from: `<config_dir>/realm/owner.key.json`
- Builds artifact at: `<path>/target/wasm32-wasip1/<profile>/<package>.wasm` (or `${CARGO_TARGET_DIR}/wasm32-wasip1/...` if set)

Platform examples for `<config_dir>`:

- Linux: `~/.config`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### Examples

Build and deploy the current package in release mode targeting peers with `edge` tag:

```
realm deploy-component --path . --profile release --tag edge
```

### See Also

- `realm-push(1)` for deploying an already-built `.wasm`
- `cargo-component` documentation for build requirements

