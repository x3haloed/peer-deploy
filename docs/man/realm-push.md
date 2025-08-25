## realm push

Push a WASI component to selected peers and optionally start it.

### Name

realm push - deploy a single WASI component to peers with signed provenance

### Overview

Reads your operator key, digests and signs the component, and publishes a deployment command over P2P. Agents that match your targeting (peers/tags) reconcile the desired state and start replicas as requested.

### Synopsis

```
realm push --name <NAME> --file <PATH> [--replicas <INT>] [--memory-max-mb <INT>] [--fuel <INT>] [--epoch-ms <INT>] [--mount <SPEC> ...] [--port <SPEC> ...] [--visibility <local|public>] [--peer <PEER_ID> ...] [--tag <TAG> ...] [--start|--no-start]
```

### Options

- `--name <NAME>`: Component name.
- `--file <PATH>`: Path to component `.wasm`.
- `--replicas <INT>`: Number of replicas. Default: 1.
- `--memory-max-mb <INT>`: Memory limit in MB. Default: 64.
- `--fuel <INT>`: WASM fuel. Default: 5000000.
- `--epoch-ms <INT>`: Epoch deadline interval in ms. Default: 100.
- `--mount <SPEC>`: Repeatable preopen mount: `host=/abs/path,guest=/www[,ro=true]`.
- `--port <SPEC>`: Repeatable service port, e.g. `8080/tcp` or `9090/udp`.
- `--visibility <local|public>`: Gateway bind policy.
- `--peer <PEER_ID>`: Target specific peers. Repeatable.
- `--tag <TAG>`: Target peers by tag/role. Repeatable.
- `--start` / `--no-start`: Start immediately (default true).

Notes: `--route-static` is deprecated and removed (HTTP served inside components via WASI HTTP).

### Files

- Reads the signing owner key from: `<config_dir>/realm/owner.key.json`
  - Linux: `~/.config/realm/owner.key.json`
  - macOS: `~/Library/Application Support/realm/owner.key.json`
  - Windows: `%APPDATA%\realm\owner.key.json`

Remote agents persist artifacts and manifests under their own data dirs (see `deploy-package` for typical paths).

### Examples

- Push a service to all peers tagged `edge` with two replicas and a TCP port:

```
realm push --name www --file ./www.wasm --replicas 2 --tag edge --port 8080/tcp
```

- Add a read-only static mount:

```
realm push --name www --file ./www.wasm --mount host=/srv/www,guest=/www,ro=true
```

### See Also

- `realm-deploy-component(1)` to build and push from Cargo
- `realm-deploy-package(1)` to install a packaged bundle


