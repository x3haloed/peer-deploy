## realm package create

Create a `.realm` bundle from a directory.

### Name

realm package create - produce a portable component bundle with assets and manifest

### Synopsis

```
realm package create [--dir <DIR>] [--name <NAME>] [--output <FILE>]
```

### Options

- `--dir <DIR>`: Directory containing `component.wasm` and optional `static/`, `config/`, `seed-data/`. Default: `.`
- `--name <NAME>`: Override component name used in manifest.
- `--output <FILE>`: Output file path for the `.realm` bundle.

### Overview

Expects the directory to contain `component.wasm` and optional `static/`, `config/`, and `seed-data/`. Generates `manifest.toml` with mounts and an integrity hash of the WASM.

### Files

- Input directory layout:
  - `<dir>/component.wasm` (required)
  - `<dir>/static/` (optional, read-only mount → `/www`)
  - `<dir>/config/` (optional, read-only mount → `/etc/app`)
  - `<dir>/seed-data/` (optional, initial seed for a stateful volume → `/data`)
- Output bundle: `<output>` (defaults to `<dir>.realm` in parent directory)

### Examples

```
realm package create --dir ./bundle --output app.realm
```

### See Also

- `realm-deploy-package(1)` to install the bundle


