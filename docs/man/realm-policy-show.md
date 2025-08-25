## realm policy-show

Show current runtime policy (native/QEMU).

### Name

realm policy-show - print the effective execution policy (file + env overrides)

### Synopsis

```
realm policy-show
```

No options.

### Files

- Policy file: `<data_dir>/realm-agent/policy.json`
- Environment overrides: `REALM_ALLOW_NATIVE_EXECUTION`, `REALM_ALLOW_EMULATION`

Platform examples for `<data_dir>`:

- Linux: `~/.local/share`
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

### See Also

- `realm-policy-set(1)` to update the policy


