## realm storage-gc

Garbage collect storage to target total size (bytes).

### Name

realm storage-gc - remove unpinned blobs until cache meets a size target

### Synopsis

```
realm storage-gc <BYTES>
```

### Arguments

- `<BYTES>`: Target total size in bytes after GC.

### Files

- Index file (used to compute size and order): `<data_dir>/realm-agent/artifacts/index.json`
- Blob storage root: `<data_dir>/realm-agent/artifacts/blobs/sha256/`

### Examples

```
realm storage-gc 5000000000   # target 5 GiB
```


