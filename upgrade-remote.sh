#!/bin/bash
set -e

# Realm Remote CI/CD Upgrade Script
# Automatically builds and deploys the latest peer-deploy to a remote Debian server

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Configuration - Update these for your setup
REMOTE_PEER_ID="12D3KooWNXg8GbGBoS3c2XpdbCJAEXM6TbDUphn6RPxCYZeGyrgw"
REMOTE_ADDRESS="/ip4/192.168.128.93/tcp/39143/p2p/$REMOTE_PEER_ID"

echo "🚀 Realm Remote CI/CD Upgrade Script"
echo "====================================="
echo "Target: $REMOTE_PEER_ID"
echo ""

# Step 1: Create fresh source tarball
echo "📦 Creating source tarball..."
tar --exclude='target' --exclude='.git' --exclude='*.tar.gz' --exclude='upgrade-remote.sh' -czf workspace.tar.gz .
echo "✅ Created workspace.tar.gz ($(du -h workspace.tar.gz | cut -f1))"

# Step 2: Ensure we're connected to the remote peer
echo ""
echo "🔗 Connecting to remote peer..."
if ! realm configure --bootstrap "$REMOTE_ADDRESS" 2>/dev/null; then
    echo "ℹ️  Bootstrap connection attempted (may already be connected)"
fi

# Wait a moment for connection to establish
sleep 2

# Step 3: Submit the build job (attach source tarball)
echo ""
echo "🏗️  Submitting build job..."
BUILD_JOB_OUTPUT=$(realm job submit build-job.toml --asset workspace.tar.gz)
echo "$BUILD_JOB_OUTPUT"
# Fallback: read last submitted job from JSON listing with filter
BUILD_JOB_ID=$(realm job list-json --status pending --limit 5 2>/dev/null | jq -r '.[0].id // empty')
if [ -z "$BUILD_JOB_ID" ]; then
  BUILD_JOB_ID=$(realm job list-json --status running --limit 5 2>/dev/null | jq -r '.[0].id // empty')
fi

if [ -z "$BUILD_JOB_ID" ]; then
    echo "❌ Failed to extract build job ID from: $BUILD_JOB_OUTPUT"
    exit 1
fi

echo "✅ Build job submitted: $BUILD_JOB_ID"

# Step 4: Wait for build to complete and show progress
echo ""
echo "⏳ Waiting for build to complete..."
echo "   (You can also run: realm job logs $BUILD_JOB_ID)"

while true; do
    STATUS=$(realm job status-json "$BUILD_JOB_ID" 2>/dev/null | jq -r '.status // "unknown"' 2>/dev/null || echo "unknown")
    
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
# Discover the built artifact name with digest and reuse it
ART_NAME=$(realm job artifacts "$BUILD_JOB_ID" | awk 'NR>3{print $1; exit}')
if [ -z "$ART_NAME" ]; then ART_NAME="realm-linux-x86_64"; fi
UPGRADE_JOB_OUTPUT=$(realm job submit upgrade-job.toml --use-artifact "$BUILD_JOB_ID:$ART_NAME")
echo "$UPGRADE_JOB_OUTPUT"
# Fallback to latest pending/running job as ID
UPGRADE_JOB_ID=$(realm job list-json --status pending --limit 5 2>/dev/null | jq -r '.[0].id // empty')
if [ -z "$UPGRADE_JOB_ID" ]; then
  UPGRADE_JOB_ID=$(realm job list-json --status running --limit 5 2>/dev/null | jq -r '.[0].id // empty')
fi

if [ -z "$UPGRADE_JOB_ID" ]; then
    echo "❌ Failed to extract upgrade job ID from: $UPGRADE_JOB_OUTPUT"
    exit 1
fi

echo "✅ Self-upgrade job submitted: $UPGRADE_JOB_ID"

# Step 6: Wait for upgrade to complete
echo ""
echo "⏳ Waiting for self-upgrade to complete..."

while true; do
    STATUS=$(realm job status-json "$UPGRADE_JOB_ID" 2>/dev/null | jq -r '.status // "unknown"' 2>/dev/null || echo "unknown")
    
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
    echo "   • Download artifacts: realm job download --job $BUILD_JOB_ID --artifact realm-linux-x86_64"
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
