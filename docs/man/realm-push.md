## realm push

Push a WASI component to selected peers and optionally start it.

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

Notes: `--route-static` is deprecated and hidden.


