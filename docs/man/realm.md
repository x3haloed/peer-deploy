## realm

peer-deploy unified agent and CLI.

### Synopsis

```
realm [OPTIONS] [COMMAND]
```

- Without a subcommand, `realm` starts the local agent. Global options influence the agent runtime.

### Global Options

- `--wasm <PATH>`: Optional WASM module path to run at startup.
- `--memory-max-mb <INT>`: Maximum memory in MB for WASM. Default: 64.
- `--fuel <INT>`: Initial fuel units for WASM. Default: 5000000.
- `--epoch-ms <INT>`: Epoch deadline interval in milliseconds. Default: 100.
- `--role <STRING>`: Role/tag to advertise. Repeatable.

### Commands

- `init`: Generate local owner key.
- `key-show`: Display owner public key.
- `apply`: Send hello / run a WASM / publish a manifest.
- `status`: Query status from agents and print first reply.
- `install`: Install the agent as a service (Unix-only).
- `upgrade`: Push an agent binary upgrade to peers.
- `push`: Push a WASI component to peers and optionally start it.
- `deploy-package`: Deploy a `.realm` package locally.
- `invite`: Create a signed invite token.
- `enroll`: Enroll a new peer using an invite token.
- `configure`: Manually configure trust and bootstrap peers.
- `diag-quic`: Diagnose a QUIC dial to a multiaddr.
- `whoami`: Print identities (CLI owner key, agent trusted owner, agent PeerId).
- `deploy-component`: Build a cargo-component and push to agents.
- `package <SUBCOMMAND>`: Package-related commands.
- `job <SUBCOMMAND>`: Job orchestration commands.
- `p2p <SUBCOMMAND>`: P2P utilities.
- `manage`: Start management web interface.
- `policy-show`: Show current runtime policy (native/QEMU).
- `policy-set`: Set runtime policy flags.
- `storage-ls`: List stored blobs (CAS).
- `storage-pin`: Pin or unpin a blob.
- `storage-gc`: Garbage collect storage to target total size.

See the respective manpages in this folder for detailed options per command.


