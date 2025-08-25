## realm configure

Manually configure trust and bootstrap peers.

### Name

realm configure - persist trusted owner key and bootstrap peer addresses for the agent

### Overview

Writes the trusted owner (public key) and a list of bootstrap multiaddrs into the agent’s data directory used by both the running agent and CLI helpers. Optionally honors `REALM_LISTEN_PORT` to persist a desired UDP listen port for QUIC.

### Synopsis

```
realm configure --owner <OWNER_KEY> [--bootstrap <MULTIADDR> ...]
```

### Options

- `--owner <OWNER_KEY>`: Owner public key (base58).
- `--bootstrap <MULTIADDR>`: Repeatable bootstrap addresses.

### Files

- Trusted owner: `<data_dir>/realm-agent/owner.pub`
- Bootstrap list: `<data_dir>/realm-agent/bootstrap.json`
- Optional listen port: `<data_dir>/realm-agent/listen_port` (when `REALM_LISTEN_PORT` is set)

Platform examples for `<data_dir>`:

- Linux: `~/.local/share`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### Examples

```
realm configure --owner $(realm key-show) --bootstrap /ip4/1.2.3.4/udp/4001/quic-v1
```

Persist a custom UDP port for QUIC:

```
REALM_LISTEN_PORT=4501 realm configure --owner $(realm key-show)
```

### See Also

- `realm-init(1)` to create your owner key
- `realm-enroll(1)` to apply an invite and optionally install the agent


