## realm status

Query status from agents and print the first reply.

### Name

realm status - quick health probe of reachable peers

### Overview

Discovers peers via configured bootstraps or mDNS, then queries for a status snapshot (software version, roles, components/jobs summary). The first response is printed and the command exits.

### Synopsis

```
realm status
```

### Description

Use this as a fast connectivity and liveness check. For richer, continuous views, use the web UI via `realm manage`.

### Examples

```
realm status
```

### See Also

- `realm-manage(1)` for the web UI
- `realm-apply(1)` for a lightweight network hello


