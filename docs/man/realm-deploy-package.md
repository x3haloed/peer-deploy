## realm deploy-package

Deploy a `.realm` package locally (installs and starts component).

### Name

realm deploy-package - install a packaged component bundle with assets and mounts

### Overview

Installs a `.realm` (zip) package by staging it under the agent data directory, extracting the embedded manifest and files, validating checksums, resolving mounts, and upserting the component into the supervisor for immediate start.

### Synopsis

```
realm deploy-package --file <PATH> [--name <NAME>]
```

### Options

- `--file <PATH>`: Path to `.realm` file. Required.
- `--name <NAME>`: Optional name override.

### Files

Staging and persistent locations are under the agent data directory:

- Packages: `<data_dir>/realm-agent/artifacts/packages/<digest>/package.zip`
- Extracted package contents: `<data_dir>/realm-agent/artifacts/packages/<digest>/...`
- Staged component wasm: `<data_dir>/realm-agent/artifacts/<name>-<digest16>.wasm`
- Persistent desired manifest: `<data_dir>/realm-agent/desired_manifest.toml`
- Work dir mounts: `<data_dir>/realm-agent/work/components/<name>`
- Stateful mounts: `<data_dir>/realm-agent/state/components/<volume-or-name>`

Platform examples for `<data_dir>`:

- Linux: `~/.local/share`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### Examples

```
realm deploy-package --file ./svc.realm
```

### See Also

- `realm-push(1)` to deploy a single `.wasm`
- `realm-package-create(1)` to produce a `.realm` bundle


