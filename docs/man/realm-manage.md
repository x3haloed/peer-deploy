## realm manage

Start management web interface.

### Name

realm manage - run a local web UI session backed by an ephemeral agent

### Synopsis

```
realm manage [--owner-key] [--timeout <MINS>]
```

### Options

- `--owner-key`: Authentication method (for now, always authenticates).
- `--timeout <MINS>`: Session timeout in minutes. Default: 30.

### Overview

Launches a local HTTP server for management operations (deploy, status, jobs). Spawns a temporary agent in-process tagged `ui` and tears it down when the session ends. Prints the session URL and ID.

### Files

- Uses the running agent’s state in memory. Some operations (e.g., install, deploy) write to the agent data directory: `<data_dir>/realm-agent/...`

### Examples

```
realm manage --timeout 60
```

### See Also

- `realm-status(1)` and `realm-push(1)` for CLI alternatives

