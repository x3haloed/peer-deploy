## realm storage-pin

Pin or unpin a blob.

### Name

realm storage-pin - protect or release cached artifacts from GC

### Synopsis

```
realm storage-pin <DIGEST> --pinned <true|false>
```

### Arguments

- `<DIGEST>`: Content digest to pin/unpin.

### Options

- `--pinned <true|false>`: Whether the blob should be pinned.

### Files

- Index file (pin metadata): `<data_dir>/realm-agent/artifacts/index.json`
- Blob path for `<DIGEST>`: `<data_dir>/realm-agent/artifacts/blobs/sha256/aa/bb/<DIGEST>`

### Examples

```
realm storage-pin sha256:deadbeef... --pinned true
```


