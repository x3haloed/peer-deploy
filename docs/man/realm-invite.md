## realm invite

Create a signed invite token for bootstrapping a new peer.

### Name

realm invite - generate a time-limited, signed bootstrap token

### Overview

The invite encodes your public key, optional realm-id, bootstrap peer addresses, and an optional expiration. The token is signed with your private owner key to prevent tampering. Share the token with the target peer to enroll.

### Synopsis

```
realm invite [--bootstrap <MULTIADDR> ...] [--realm-id <ID>] [--exp-mins <INT>]
```

### Options

- `--bootstrap <MULTIADDR>`: Repeatable bootstrap multiaddr(s) to embed.
- `--realm-id <ID>`: Realm identifier to embed.
- `--exp-mins <INT>`: Expiration minutes. Default: 60. Use 0 for no expiration.

### Files

- Reads the signing owner key from: `<config_dir>/realm/owner.key.json`

Platform examples for `<config_dir>`:

- Linux: `~/.config`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### Examples

```
realm invite --bootstrap /ip4/1.2.3.4/udp/4001/quic-v1 --exp-mins 30 > token.txt
```

### See Also

- `realm-enroll(1)` to use the token
- `realm-configure(1)` for manual configuration


