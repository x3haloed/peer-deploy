## realm enroll

Enroll a new peer using an invite token; optionally install the agent.

### Name

realm enroll - apply a signed invite, persist configuration, and optionally install the agent

### Overview

Validates the invite signature, expiration, and contents. Persists the trusted owner and bootstrap addresses locally. If provided a binary, installs the agent (user or system service); otherwise prints next steps.

### Synopsis

```
realm enroll --token <TOKEN> [--binary <PATH>] [--system]
```

### Options

- `--token <TOKEN>`: Invite token (base64). Required.
- `--binary <PATH>`: Path to agent binary to install (optional).
- `--system`: Install as a system service (optional, Unix-only).

### Files

- Trusted owner: `<data_dir>/realm-agent/owner.pub`
- Bootstrap list: `<data_dir>/realm-agent/bootstrap.json`

If installing immediately, see `realm install` for service file locations.

Platform examples for `<data_dir>`:

- Linux: `~/.local/share`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### Examples

Enroll and install as a system service (Linux):

```
realm enroll --token $(cat token.txt) --binary ./agent-linux --system
```

Enroll only; install later:

```
realm enroll --token $(cat token.txt)
realm install --binary ./agent-linux
```

### See Also

- `realm-invite(1)` to generate the token
- `realm-install(1)` to install the agent


