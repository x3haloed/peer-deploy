## realm whoami

Print identities: CLI owner key, agent trusted owner, agent PeerId.

### Name

realm whoami - show local CLI identity and agent identity/trust configuration

### Synopsis

```
realm whoami
```

No options.

### Files

- CLI owner key: `<config_dir>/realm/owner.key.json`
- Agent trusted owner: `<data_dir>/realm-agent/owner.pub`
- Agent node PeerId (if present): `<data_dir>/realm-agent/node.peer`

Platform examples for `<config_dir>` and `<data_dir>`:

- Linux: `~/.config` and `~/.local/share`
- macOS: `~/Library/Application Support` for both
- Windows: `%APPDATA%` for both

### See Also

- `realm-init(1)`, `realm-configure(1)`


