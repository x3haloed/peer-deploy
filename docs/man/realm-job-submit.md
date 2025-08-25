## realm job submit

Submit a job from a TOML specification.

### Name

realm job submit - stage assets, submit a job spec, and broadcast to peers

### Synopsis

```
realm job submit <FILE> [--asset <NAME=PATH|PATH> ...] [--use-artifact <JOBID:NAME> ...]
```

### Arguments

- `<FILE>`: Path to job TOML file.

### Options

- `--asset <NAME=PATH|PATH>`: Attach a local file as an asset (repeatable).
- `--use-artifact <JOBID:NAME>`: Reuse an artifact from a completed job (repeatable).

### Files

- Reads CLI owner key to ensure identity: `<config_dir>/realm/owner.key.json`
- Stages assets into local CAS: `<data_dir>/realm-agent/artifacts/blobs/sha256/...`
- Job state directory: `<data_dir>/realm-agent/jobs/`

### Description

Small assets are inlined over P2P; large ones are chunked. Assets are also stored locally in CAS and referenced via `cas:<digest>` in pre-stage steps. Reused artifacts are looked up from prior jobs and added as pre-stage entries.

### Examples

```
realm job submit ./jobs/resize.toml --asset image=./photo.jpg
```

Reuse an artifact from a previous job:

```
realm job submit ./jobs/analyze.toml --use-artifact 01H..XYZ:report.json
```


