## realm upgrade

Push an agent binary upgrade to peers.

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


