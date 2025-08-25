## realm deploy-component

Build a cargo-component and push to agents.

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


