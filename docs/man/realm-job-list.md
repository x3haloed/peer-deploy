## realm job list

List all jobs (running, scheduled, completed).

### Name

realm job list - list local job records with optional refresh from peers

### Synopsis

```
realm job list [--status <STATUS>] [--limit <INT>] [--fresh]
```

### Options

- `--status <STATUS>`: Filter by status (pending, running, completed, failed).
- `--limit <INT>`: Maximum number of jobs to show. Default: 50.
- `--fresh`: Refresh job state from peers before listing.

### Files

- Job state directory: `<data_dir>/realm-agent/jobs/`

### Examples

```
realm job list --status running --limit 100 --fresh
```


