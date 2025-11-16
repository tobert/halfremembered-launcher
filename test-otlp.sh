#!/usr/bin/env bash
# Quick OTLP testing script
#
# Usage:
#   ./test-otlp.sh          # Test with otlp-mcp on localhost:4317
#   ./test-otlp.sh custom   # Test with custom endpoint

set -e

OTLP_ENDPOINT="${1:-http://localhost:4317}"

echo "🧪 Testing OTLP integration with endpoint: $OTLP_ENDPOINT"
echo ""

# Build if needed
if [ ! -f target/release/halfremembered-launcher ]; then
    echo "📦 Building release version..."
    cargo build --release
    echo ""
fi

# Check if we're in a project directory with config
if [ ! -f .hrlauncher.toml ]; then
    echo "⚠️  No .hrlauncher.toml found in current directory"
    echo "   Starting server anyway (no file watching)..."
    echo ""
fi

echo "🚀 Starting server with OTLP enabled..."
echo "   Press Ctrl+C to stop"
echo ""

# Start server with OTLP
exec ./target/release/halfremembered-launcher server --otlp-endpoint "$OTLP_ENDPOINT"
