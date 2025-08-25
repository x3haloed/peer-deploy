## realm upgrade

Push an agent binary upgrade to peers.

### Name

realm upgrade - publish agent binaries to selected peers and roll forward versions

### Synopsis

```
realm upgrade [--bin <PLAT=PATH> ...] [--file <PATH>] [--platform <PLATFORM>] [--all-platforms] [--version <INT>] [--peer <PEER_ID> ...] [--tag <TAG> ...]
```

### Options

- `--bin <PLAT=PATH>`: Repeatable platform=path pair (e.g., `linux/x86_64=./agent-linux`). Platform can be auto-detected if omitted.
- `--file <PATH>`: Legacy single-binary path.
- `--platform <PLATFORM>`: Legacy explicit platform for `--file`.
- `--all-platforms`: Publish all provided binaries to their matching platforms.
- `--version <INT>`: Version to publish. Default: 1.
- `--peer <PEER_ID>`: Target specific peers by PeerId. Repeatable.
- `--tag <TAG>`: Target peers by tag/role. Repeatable.

### Description

Use multi-`--bin` for cross-platform fleets; `--all-platforms` ensures each platform receives its matching binary. Versioning helps coordinate staged rollouts. You can target by exact PeerId or by role tags.

### Files

- Reads the signing owner key from: `<config_dir>/realm/owner.key.json`
  - Linux: `~/.config/realm/owner.key.json`
  - macOS: `~/Library/Application Support/realm/owner.key.json`
  - Windows: `%APPDATA%\realm\owner.key.json`

### Examples

```
realm upgrade --bin linux/x86_64=./agent-linux --bin aarch64-apple-darwin=./agent-macos \
  --all-platforms --version 3 --tag edge --tag core
```

### See Also

- `realm-install(1)` to install the agent locally
- `realm-manage(1)` for monitoring

