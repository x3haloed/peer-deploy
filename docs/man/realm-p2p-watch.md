## realm p2p watch

Watch all P2P messages in real time.

### Name

realm p2p watch - subscribe to command and status topics and print messages

### Synopsis

```
realm p2p watch
```

No options.

### Description

Connects to peers (using bootstrap/mDNS), subscribes to the command and status gossip topics, and prints a rate-limited preview of messages. Useful for debugging control-plane activity.

### Files

- Bootstrap configuration may be used: `<data_dir>/realm-agent/bootstrap.json`

### Examples

```
realm p2p watch
```


