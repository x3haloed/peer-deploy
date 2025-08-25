## realm job download

Download an artifact from a specific job.

### Name

realm job download - copy a staged artifact to a local file path

### Synopsis

```
realm job download <JOB_ID> <ARTIFACT_NAME> [-o|--output <PATH>]
```

### Arguments

- `<JOB_ID>`: Job ID.
- `<ARTIFACT_NAME>`: Artifact name.

### Options

- `-o`, `--output <PATH>`: Output file path (optional, defaults to artifact name).

### Files

- Job state directory (staged artifacts): `<data_dir>/realm-agent/jobs/`


