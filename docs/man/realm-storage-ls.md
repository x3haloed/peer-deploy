## realm storage-ls

List stored blobs (CAS).

### Name

realm storage-ls - list content-addressable artifacts in the agent cache

### Synopsis

```
realm storage-ls
```

No options.

### Files

- Blob storage root: `<data_dir>/realm-agent/artifacts/blobs/sha256/aa/bb/<digest>`
- Index file: `<data_dir>/realm-agent/artifacts/index.json`

Platform examples for `<data_dir>`:

- Linux: `~/.local/share`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### See Also

- `realm-storage-pin(1)` to pin/unpin
- `realm-storage-gc(1)` to garbage-collect


