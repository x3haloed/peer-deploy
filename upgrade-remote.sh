#!/bin/bash
set -e

# Realm Remote CI/CD Upgrade Script
# Automatically builds and deploys the latest peer-deploy to a remote Debian server

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🚀 Realm Remote CI/CD Upgrade Script"
echo "====================================="
echo ""

# Resolve realm CLI from local repo by default to ensure latest features
REALM_BIN=${REALM_BIN:-"./target/release/realm"}
if [ ! -x "$REALM_BIN" ]; then
  if command -v cargo >/dev/null 2>&1; then
    echo "🔧 Building realm CLI..."
    cargo build --release --bin realm || {
      echo "❌ Failed to build realm CLI. Install Rust and try again." >&2; exit 1; }
  else
    echo "❌ Rust toolchain not found and REALM_BIN not set. Install Rust or set REALM_BIN to a realm binary." >&2
    exit 1
  fi
fi

# Validate required subcommands and flags
if ! "$REALM_BIN" --help 2>/dev/null | grep -q " job "; then
  echo "❌ realm CLI missing 'job' subcommands. Ensure REALM_BIN points to this repo's built CLI." >&2
  exit 1
fi
if ! "$REALM_BIN" job submit --help 2>/dev/null | grep -q -- "--asset"; then
  echo "❌ realm CLI missing '--asset' support. Please rebuild from this repo (cargo build --release)." >&2
  exit 1
fi

# Target selection via tags (jobs specify tags = ["dev"]) rather than explicit peer IDs
echo "ℹ️  Jobs will target peers tagged 'dev' via job targeting."

# Step 1: Create fresh source tarball
echo "📦 Creating source tarball..."
tar --exclude='target' --exclude='.git' --exclude='*.tar.gz' --exclude='upgrade-remote.sh' -czf workspace.tar.gz .
echo "✅ Created workspace.tar.gz ($(du -h workspace.tar.gz | cut -f1))"

# Step 2: Ensure we're connected to the remote peer
echo ""
echo "🔗 Ensuring discovery is warm..."
# Best-effort: try a status call to warm up local discovery cache (optional)
$REALM_BIN status >/dev/null 2>&1 || true

# Wait a moment for connection to establish
sleep 2

# Step 3: Submit the build job (attach source tarball)
echo ""
echo "🏗️  Submitting build job..."
BUILD_JOB_OUTPUT=$($REALM_BIN job submit build-job.toml --asset workspace.tar.gz)
echo "$BUILD_JOB_OUTPUT"
# Resolve job ID by querying the network (retry with longer timeout for network sync)
echo "🔎 Resolving build job ID from network..."
BUILD_JOB_ID=""
for i in $(seq 1 30); do
  # Look for the most recently submitted build job that's still active (exclude completed/failed/cancelled)
  BUILD_JOB_ID=$($REALM_BIN job net-list-json --limit 100 2>/dev/null | jq -r 'map(select(.spec.name=="build-peer-deploy" and (.status == "pending" or .status == "running" or .status == "started"))) | sort_by(.submitted_at) | reverse | (.[0].id // empty)')
  [ -n "$BUILD_JOB_ID" ] && break
  echo "   Waiting for active job to appear in network... (attempt $i/30)"
  sleep 2
done

if [ -z "$BUILD_JOB_ID" ]; then
    echo "❌ Failed to resolve build job ID from network."
    exit 1
fi

echo "✅ Build job submitted: $BUILD_JOB_ID"

# Step 4: Wait for build to complete and show progress
echo ""
echo "⏳ Waiting for build to complete..."
echo "   (You can also run: $REALM_BIN job logs $BUILD_JOB_ID)"

while true; do
    STATUS=$($REALM_BIN job net-status-json "$BUILD_JOB_ID" 2>/dev/null | jq -r '.status // "unknown"' 2>/dev/null || echo "unknown")
    
    case "$STATUS" in
        "completed")
            echo "✅ Build completed successfully!"
            break
            ;;
        "failed")
            echo "❌ Build failed. Check logs with: realm job logs $BUILD_JOB_ID"
            exit 1
            ;;
        "cancelled")
            echo "❌ Build was cancelled"
            exit 1
            ;;
        *)
            echo "⏳ Build status: $STATUS (waiting...)"
            sleep 10
            ;;
    esac
done

# Step 5: Submit the self-upgrade job reusing the built artifact
echo ""
echo "🔄 Submitting self-upgrade job..."
# Discover the built artifact name and reuse it (via network status)
ART_NAME=$($REALM_BIN job net-status-json "$BUILD_JOB_ID" 2>/dev/null | jq -r '.artifacts[0].name // empty')
if [ -z "$ART_NAME" ]; then ART_NAME="realm-linux-x86_64"; fi
UPGRADE_JOB_OUTPUT=$($REALM_BIN job submit upgrade-job.toml --use-artifact "$BUILD_JOB_ID:$ART_NAME")
echo "$UPGRADE_JOB_OUTPUT"
# Resolve upgrade job ID by querying the network (retry with longer timeout)
UPGRADE_JOB_ID=""
for i in $(seq 1 30); do
  # Look for the most recently submitted upgrade job that's still active (exclude completed/failed/cancelled)
  UPGRADE_JOB_ID=$($REALM_BIN job net-list-json --limit 100 2>/dev/null | jq -r 'map(select(.spec.name=="self-upgrade-agent" and (.status == "pending" or .status == "running"))) | sort_by(.submitted_at) | reverse | (.[0].id // empty)')
  [ -n "$UPGRADE_JOB_ID" ] && break
  echo "   Waiting for active upgrade job to appear in network... (attempt $i/30)"
  sleep 2
done

if [ -z "$UPGRADE_JOB_ID" ]; then
    echo "❌ Failed to extract upgrade job ID from: $UPGRADE_JOB_OUTPUT"
    exit 1
fi

echo "✅ Self-upgrade job submitted: $UPGRADE_JOB_ID"

# Step 6: Wait for upgrade to complete
echo ""
echo "⏳ Waiting for self-upgrade to complete..."

while true; do
    STATUS=$($REALM_BIN job net-status-json "$UPGRADE_JOB_ID" 2>/dev/null | jq -r '.status // "unknown"' 2>/dev/null || echo "unknown")
    
    case "$STATUS" in
        "completed")
            echo "✅ Self-upgrade completed successfully!"
            break
            ;;
        "failed")
            echo "❌ Self-upgrade failed. Check logs with: realm job logs $UPGRADE_JOB_ID"
            exit 1
            ;;
        "cancelled")
            echo "❌ Self-upgrade was cancelled"
            exit 1
            ;;
        *)
            echo "⏳ Upgrade status: $STATUS (waiting...)"
            sleep 5
            ;;
    esac
done

# Step 7: Verify the upgrade
echo ""
echo "🔍 Verifying remote agent status..."
sleep 3

if realm status >/dev/null 2>&1; then
    echo "✅ Remote agent is responding!"
    echo ""
    echo "🎉 UPGRADE COMPLETE! 🎉"
    echo ""
    echo "📊 Job Summary:"
    echo "   Build Job:   $BUILD_JOB_ID"
    echo "   Upgrade Job: $UPGRADE_JOB_ID"
    echo ""
    echo "💡 Pro Tips:"
    echo "   • View logs: realm job logs <job-id>"
    echo "   • Download artifacts: $REALM_BIN job download $BUILD_JOB_ID realm-linux-x86_64"
    echo "   • Check status: realm status"
else
    echo "⚠️  Remote agent may still be restarting..."
    echo "   Wait a moment and try: realm status"
fi

# Cleanup
echo ""
echo "🧹 Cleaning up..."
rm -f workspace.tar.gz
echo "✅ Cleanup complete"

echo ""
echo "🚀 Remote CI/CD upgrade workflow completed!"

