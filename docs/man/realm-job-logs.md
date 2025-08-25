## realm job logs

Show logs for a specific job.

### Name

realm job logs - print or follow job logs from local job state

### Synopsis

```
realm job logs <JOB_ID_OR_NAME> [--tail <INT>] [-f|--follow]
```

### Arguments

- `<JOB_ID_OR_NAME>`: Job ID or name to show logs for.

### Options

- `--tail <INT>`: Number of recent log lines to show. Default: 100.
- `-f`, `--follow`: Follow log output in real-time.

### Files

- Job state directory: `<data_dir>/realm-agent/jobs/`


