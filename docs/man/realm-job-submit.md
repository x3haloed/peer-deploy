## realm job submit

Submit a job from a TOML specification.

### Synopsis

```
realm job submit <FILE> [--asset <NAME=PATH|PATH> ...] [--use-artifact <JOBID:NAME> ...]
```

### Arguments

- `<FILE>`: Path to job TOML file.

### Options

- `--asset <NAME=PATH|PATH>`: Attach a local file as an asset (repeatable).
- `--use-artifact <JOBID:NAME>`: Reuse an artifact from a completed job (repeatable).


