## realm policy-set

Set runtime policy flags.

### Name

realm policy-set - persist execution policy toggles

### Synopsis

```
realm policy-set [--native <true|false>] [--qemu <true|false>]
```

### Options

- `--native <true|false>`: Allow native execution.
- `--qemu <true|false>`: Allow QEMU emulation.

### Files

- Writes policy to: `<data_dir>/realm-agent/policy.json`
- Environment variables can override at runtime: `REALM_ALLOW_NATIVE_EXECUTION`, `REALM_ALLOW_EMULATION`

### Examples

```
realm policy-set --native true --qemu true
```

### See Also

- `realm-policy-show(1)` to view current policy

