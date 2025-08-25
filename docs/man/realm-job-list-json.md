## realm job list-json

List jobs as JSON (for scripts).

### Name

realm job list-json - print jobs in JSON from local state

### Synopsis

```
realm job list-json [--status <STATUS>] [--limit <INT>]
```

### Options

- `--status <STATUS>`: Filter by status (pending, running, completed, failed).
- `--limit <INT>`: Maximum number of jobs to show. Default: 50.

### Files

- Job state directory: `<data_dir>/realm-agent/jobs/`


