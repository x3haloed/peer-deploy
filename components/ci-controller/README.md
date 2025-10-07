# CI Controller (WASI HTTP component)

A tiny Realm component that listens for GitHub webhooks and triggers builds on your Realm mesh.

## What it does
- Exposes an HTTP endpoint at `/ci-controller/hook` via the Realm gateway
- Verifies GitHub `X-Hub-Signature-256` if you mount a secret
- Triggers on release-tag events:
  - `release` events: uses `release.tag_name`
  - `push` events: detects tag pushes (`ref = refs/tags/<tag>`)
- Fetches a source tarball for `{owner}/{repo}` at the release tag from `https://codeload.github.com/{owner}/{repo}/tar.gz/{tag}`
- Submits a Realm Job to build the project, attaching the tarball as `workspace.tar.gz`

## Prerequisites
- Realm agent running on an edge node (to expose a public gateway) with the `edge` role
- Policy allowing native execution for build jobs (if you use the provided native build script)
- Optional: a GitHub webhook secret stored on disk

## Build the component
```bash
# From repo root
cargo component build -p ci-controller --features component
# Output: target/wasm32-wasi/release/ci_controller.wasm
```

## Deploy the component
Basic deploy (local visibility):
```bash
realm push \
  --name ci-controller \
  --file target/wasm32-wasi/release/ci_controller.wasm \
  --replicas 1 \
  --memory-max-mb 64 \
  --epoch-ms 100 \
  --tag dev \
  --start
```

Expose publicly on the gateway (requires an agent with `edge` role):
```bash
realm push \
  --name ci-controller \
  --file target/wasm32-wasi/release/ci_controller.wasm \
  --visibility public \
  --replicas 1 \
  --memory-max-mb 64 \
  --epoch-ms 100 \
  --tag edge \
  --start
```

Endpoint will be available at:
- Local: `http://127.0.0.1:8080/ci-controller/hook`
- Public (edge): `http://<edge-node>:8080/ci-controller/hook`

## Optional mounts
To verify webhook signatures and/or provide a fallback workspace bundle:
- Secret: mount file at `/config/secret`
- Workspace tarball: mount file at `/workspace/workspace.tar.gz`
- Platforms config: `/config/platforms.json` (JSON array of strings) or `/config/platforms.txt` (CSV/newline list)
- GitHub token (for uploading assets to release): `/config/github_token` (plain text)

Mounts are specified when you package/deploy the component (add to `--mount` in CLI or Manifest mounts).

## Configure GitHub webhook
- In your repository settings → Webhooks → Add webhook
- Payload URL: `http://<edge-node>:8080/ci-controller/hook`
- Content type: `application/json`
- Secret: set a strong secret and place the same in the agent filesystem at the mounted path (`/config/secret`)
- Which events: choose "Let me select individual events" and enable:
  - `Release`
  - `Push` (if you want to trigger on tags pushed)

## What the controller submits
A job equivalent to:
- Runtime: native `/usr/bin/bash`
- Steps: extract `workspace.tar.gz` to `/tmp/workspace` and run `cargo build --release --bin realm`
- Captures artifact `target/release/realm` as `realm-binary`
- Targeting: `linux/x86_64` (adjust as needed)

If `/config/github_token` is present, the controller passes it to the build job as an attachment (`/tmp/assets/gh_token`). The job will look up the GitHub release by tag and upload the built artifact to that release's assets.

You can customize `components/ci-controller/src/lib.rs` to:
- Add more platforms (`linux/aarch64`) and submit multiple jobs
- Chain an upgrade job using the produced artifact
- Change build commands or capture additional artifacts

## Troubleshooting
- Logs: open Realm Web UI → Jobs → follow logs for the submitted job
- Policy: ensure native execution is enabled (Web UI Ops → Policy)
- Gateway: ensure an `edge` node is running for public exposure
- Signature: if you configured a secret, mismatches will return 401

## Security notes
- Manual trigger
  - You can manually trigger builds without a webhook:
    - `GET http://<node>:8080/ci-controller/manual?repo=<owner>/<repo>&tag=<tag>&platforms=linux/x86_64,linux/aarch64`
    - If `platforms` is omitted, it uses `/config/platforms.json` or `/config/platforms.txt` (defaults to `linux/x86_64`).
- HMAC verification is enforced only if `/config/secret` is mounted
- Consider running behind a reverse proxy with TLS if internet-exposed

