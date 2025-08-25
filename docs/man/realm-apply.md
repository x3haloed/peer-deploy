## realm apply

Send a hello to the network, run a WASM component once, or publish a manifest.

### Name

realm apply - ad-hoc apply of a WASM component or manifest version

### Overview

`apply` is a versatile operation:

- With no flags, broadcast a discovery/hello to connected peers and print the first response.
- With `--wasm`, run a provided WASM component immediately (one-shot) on the local agent context.
- With `--file`, publish a TOML manifest describing components to reconcile, tagged with `--version`.

### Synopsis

```
realm apply [--wasm <PATH>] [--file <PATH>] [--version <INT>]
```

### Options

- `--wasm <PATH>`: Path to a WASM component to run locally once.
- `--file <PATH>`: Path to a manifest (.toml) to publish to peers.
- `--version <INT>`: Version number for the manifest. Default: 1.

### Examples

- Ping the network and show the first peer's status:

```
realm apply
```

- Execute a local component for a quick check:

```
realm apply --wasm ./component.wasm
```

- Publish a manifest at version 3:

```
realm apply --file ./realm.toml --version 3
```

### See Also

- `realm-push(1)` to deploy a single component
- `realm-deploy-package(1)` to install a packaged bundle


