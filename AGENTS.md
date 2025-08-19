# Interface Parity and UX Consistency

Maintain feature parity and a consistent UX across the three user surfaces:

- CLI flags and subcommands (see [crates/agent/src/main.rs](crates/agent/src/main.rs), [crates/agent/src/cmd](crates/agent/src/cmd))
- Web UI forms and operations (see [crates/agent/web/index.html](crates/agent/web/index.html), [crates/agent/web/app.js](crates/agent/web/app.js))
- TOML manifest schema (see [crates/common/src/lib.rs](crates/common/src/lib.rs) for `Manifest`, `ComponentSpec`)

Agent-side enforcement/behavior lives in:
- [crates/agent/src/p2p/handlers.rs](crates/agent/src/p2p/handlers.rs)
- [crates/agent/src/p2p/mod.rs](crates/agent/src/p2p/mod.rs)
- [crates/agent/src/runner.rs](crates/agent/src/runner.rs)
- [crates/agent/src/supervisor.rs](crates/agent/src/supervisor.rs)

## Golden Rule
When you add or change a platform capability that configures component behavior, you MUST reflect it in all applicable interfaces:

- Update `ComponentSpec` / `Manifest` types to carry the new option.
- Add/adjust CLI options to set the value (`realm push`, `realm apply`).
- Add/adjust Web UI forms to collect the value (Deploy, Ops tabs).
- Ensure agent reads and enforces the value (handlers/supervisor/runner).
- Document briefly in `README.md`.

Examples of platform options that MUST be kept in sync:
- Resource limits (e.g., memory, fuel, epoch)
- Execution topology (replicas, selection by peer IDs/tags)
- Artifacts and integrity (source, digest)
- New runtime features (e.g., mounts, environment, args, routes)

## Allowed Exceptions (Do NOT add to TOML)
Operational actions that are not part of desired state should not appear in the TOML schema. They should exist in CLI/Web UI only.

- Installation flows (e.g., `realm install`, Web UI Ops → Install).
- One-off visualizations (e.g., Web UI component stats, real-time metrics).
- Ad-hoc queries (e.g., `realm status`, Web UI log viewing).

If a concept is visualization-only in the Web UI, consider a CLI analogue that emits data once and exits (e.g., a timestamped event dump) but do not add it to the TOML.

## Practical Checklist for Any New Capability
1. Schema: extend `ComponentSpec` (and `Manifest` if needed) in [crates/common/src/lib.rs](crates/common/src/lib.rs).
2. CLI:
   - Add flags in the relevant subcommand (e.g., [crates/agent/src/cmd/push.rs](crates/agent/src/cmd/push.rs)).
   - Serialize into the carrier type (e.g., `PushUnsigned`).
3. Web UI:
   - Add fields/forms in the appropriate tab (Deploy, Ops) in [crates/agent/web/index.html](crates/agent/web/index.html).
   - Handle form submission and API calls in [crates/agent/web/app.js](crates/agent/web/app.js).
   - Add corresponding API endpoint in [crates/agent/src/web/handlers.rs](crates/agent/src/web/handlers.rs).
4. Agent:
   - Accept and persist via handlers in [crates/agent/src/p2p/mod.rs](crates/agent/src/p2p/mod.rs) / [crates/agent/src/p2p/handlers.rs](crates/agent/src/p2p/handlers.rs).
   - Reconcile/apply in [crates/agent/src/supervisor.rs](crates/agent/src/supervisor.rs) and enforce at runtime in [crates/agent/src/runner.rs](crates/agent/src/runner.rs).
5. Docs: add a brief bullet and example in [README.md](README.md).

## Decision Guide: Should it be in TOML?
- Is it desired state for continuous reconciliation (what should be running, with which limits/resources)?
  - Yes → Add to TOML + CLI + Web UI.
  - No → Keep to CLI/Web UI ops only.

## Example: Mounts (preopened directories)
- TOML: add `mounts` to `ComponentSpec`.
- CLI: `realm push --mount host=/var/www,guest=/www,ro=true` (repeatable).
- Web UI: form fields in Deploy tab to add/edit mounts; validate host path required.
- Agent: preopen host dirs in WASI context in `runner.rs`.

## Current Interface Mapping

### Component Deployment
- **CLI**: `realm push --name X --file Y.wasm --replicas N --memory-max-mb M --fuel F --epoch-ms E --tag T`
- **Web UI**: Deploy tab form with file upload, text inputs for name/replicas/memory/fuel/epoch, tags field
- **API**: `POST /api/deploy-multipart` with multipart form data
- **TOML**: `ComponentSpec` with name, replicas, memory_max_mb, fuel, epoch_ms, target_tags

### Manifest Application
- **CLI**: `realm apply --file realm.toml --version N`
- **Web UI**: Ops tab → Apply Manifest with file upload and version input
- **API**: `POST /api/apply` with multipart form data
- **TOML**: `Manifest` containing multiple `ComponentSpec` entries

### Agent Upgrade
- **CLI**: `realm upgrade --file agent-binary --version N --tag T`
- **Web UI**: Ops tab → Upgrade Agent with file upload and version input
- **API**: `POST /api/upgrade` with multipart form data

### Peer Connection
- **CLI**: `realm configure --bootstrap <multiaddr>` (persistent) or connection via mDNS
- **Web UI**: Ops tab → Connect Peer with multiaddr input
- **API**: `POST /api/connect` with JSON body

### Status and Monitoring
- **CLI**: `realm status` (one-shot query)
- **Web UI**: Overview, Nodes, Components tabs with real-time updates via WebSocket
- **API**: `GET /api/status`, `GET /api/nodes`, `GET /api/components`, WebSocket `/ws`

## Validation & Tests
- Reject unknown/unsupported options early at the CLI/Web UI layer with clear error messages.
- Agent must validate inputs (e.g., refuse mounts outside allowed dirs if policy applies).
- Prefer schema-derived structs over ad-hoc maps to keep parity type-safe.
- Web UI should provide client-side validation before submission.
- API endpoints should validate inputs and return meaningful error responses.