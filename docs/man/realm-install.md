## realm install

Install the agent as a service (systemd user service by default). Unix-only.

### Name

realm install - set up the realm agent as a managed system service

### Overview

Installs and enables the agent to run in the background at login (user service) or at boot (system service). This is the recommended way to keep the agent available for deployments and jobs.

### Synopsis

```
realm install [--binary <PATH>] [--system]
```

### Options

- `--binary <PATH>`: Path to the agent binary to install. If omitted, uses the current executable.
- `--system`: Install as a system service instead of a user service (may require elevated privileges).

### Description

User service installs do not require root and tie the agent lifetime to the user session. System service installs start at boot and continue independently of user logins.

### Examples

- Install as a user service using the current binary:

```
realm install
```

- Install a specific binary as a system service (requires privileges):

```
realm install --binary ./agent-linux --system
```

### Files

- User service install (no root):
  - Binary store: `<data_dir>/realm-agent/bin/realm-agent-<digest16>`
  - Symlink to current: `<data_dir>/realm-agent/bin/current`
  - User PATH link: `~/.local/bin/realm-agent -> <data_dir>/realm-agent/bin/current`
  - systemd unit: `~/.config/systemd/user/realm-agent.service`
  - On Linux, `<data_dir>` typically is `~/.local/share`.
  - On macOS, `<data_dir>` typically is `~/Library/Application Support`.

- System service install (root/systemd):
  - Binary store: `/usr/local/lib/realm-agent/bin/realm-agent-<digest16>`
  - Symlink to current: `/usr/local/lib/realm-agent/bin/current`
  - PATH link: `/usr/local/bin/realm-agent -> .../current`
  - systemd unit: `/etc/systemd/system/realm-agent.service`

### See Also

- `realm-upgrade(1)` to roll out new agent binaries to peers
- `realm-manage(1)` to operate via the web UI


