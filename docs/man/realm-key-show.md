## realm key-show

Display the owner public key.

### Name

realm key-show - print the operator public key for sharing and enrollment

### Overview

Prints the public portion of the owner key created with `realm init`. This key identifies you to peers and is commonly used when bootstrapping trust, generating invites, or verifying who performed an operation.

### Synopsis

```
realm key-show
```

### Description

The output format is a textual encoding suitable for pasting into configuration or sharing with collaborators. Keep your private key safe; this command only reveals the public key.

### Examples

- Copy your public key to the clipboard (macOS):

```
realm key-show | pbcopy
```

- Use it to configure an agent's trusted owner:

```
realm key-show | xargs -I{} realm configure --owner {} --bootstrap /ip4/1.2.3.4/udp/4001/quic-v1
```

### Files

- Reads the owner key from: `<config_dir>/realm/owner.key.json`
  - Linux: `~/.config/realm/owner.key.json`
  - macOS: `~/Library/Application Support/realm/owner.key.json`
  - Windows: `%APPDATA%\realm\owner.key.json`

### See Also

- `realm-init(1)` to generate the key
- `realm-configure(1)` to set trust/bootstraps


