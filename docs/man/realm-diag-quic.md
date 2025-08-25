## realm diag-quic

Diagnose a QUIC dial attempt to a multiaddr; prints handshake results.

### Name

realm diag-quic - attempt a QUIC connection and print diagnostics

### Synopsis

```
realm diag-quic <MULTIADDR>
```

### Arguments

- `<MULTIADDR>`: Target multiaddr to connect to.

### Description

Submits a QUIC dial to the given multiaddr and waits up to 5 seconds for connection events, reporting success or error details. Useful to check connectivity, DNS, NAT, and firewall issues.

### Examples

```
realm diag-quic /ip4/203.0.113.10/udp/4001/quic-v1
```


