## realm init

Generate (or ensure) the local owner key.

### Name

realm init - create a local operator keypair used to identify and authorize actions

### Overview

The owner key is your long-lived operator identity. Peers can trust this key to authorize pushes, upgrades, and configuration. Running `realm init` is typically the first step before interacting with a realm network.

If a key already exists, the command is idempotent and will leave it unchanged.

### Synopsis

```
realm init
```

### Description

Creates a new owner keypair and stores it in the standard location for the CLI. The private key remains on your machine; only the public key is meant to be shared (e.g., to bootstrap or enroll peers).

### Examples

- Initialize once on a fresh machine:

```
realm init
```

- Show the public key after initialization:

```
realm key-show
```

### Files

- Owner key directory: platform config directory joined with `realm`.
  - Linux: `~/.config/realm`
  - macOS: `~/Library/Application Support/realm`
  - Windows: `%APPDATA%\realm`
- Private key file: `<config_dir>/realm/owner.key.json`
- Sample manifest (if a local `hello.wasm` is detected): `<config_dir>/realm/realm.sample.toml`

### See Also

- `realm-key-show(1)` to display the public key
- `realm-configure(1)` to set trust/bootstraps on an agent
- `realm-enroll(1)` to join a peer using an invite token


