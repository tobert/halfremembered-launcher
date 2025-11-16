# OTLP Integration Testing Guide

This document explains how to test the OpenTelemetry (OTLP) log forwarding feature.

## Quick Start

### 1. Start OTLP Collector (otlp-mcp or OTEL Collector)

**Option A: Using otlp-mcp**
```bash
# If you have otlp-mcp running via MCP, it should already be listening on localhost:4317
```

**Option B: Using Docker OTEL Collector**
```bash
docker run -p 4317:4317 -p 4318:4318 \
  otel/opentelemetry-collector:latest
```

### 2. Run the Server with OTLP Enabled

```bash
cd ~/src/sregame  # Or any directory with .hrlauncher.toml

# Using the test script:
./test-otlp.sh

# Or manually:
halfremembered-launcher server --otlp-endpoint http://localhost:4317
```

### 3. What to Look For

When the server starts with OTLP enabled, you should see:

```
[INFO] Initializing OTLP exporter: http://localhost:4317
[INFO] ✅ OTLP exporter initialized successfully
[INFO] ✅ OTLP logging enabled: http://localhost:4317
```

A test log will be sent immediately with:
- `request_id`: "test-init"
- `client.hostname`: "server"
- `process.binary`: "halfremembered-launcher"
- `stream.type`: "stdout"
- Body: "OTLP exporter initialized successfully"

## Current Status

### ✅ Implemented
- OTLP exporter infrastructure
- Server CLI flag (`--otlp-endpoint`)
- Structured log format with attributes
- Test initialization message

### 🚧 Work in Progress
- Client-side process stdout/stderr streaming
- Server-side exec channel handler
- Real-time log forwarding from executed processes

### 📋 TODO
- Integration tests with mock OTLP collector
- Metrics (messages sent, errors, etc.)
- Retry logic for OTLP failures
- Configurable batch settings

## Manual Testing Workflow

1. **Test initialization:**
   ```bash
   ./test-otlp.sh
   ```
   Check that OTLP exporter initializes without errors.

2. **Test without OTLP:**
   ```bash
   halfremembered-launcher server
   ```
   Server should work normally, no OTLP messages.

3. **Test with unreachable endpoint:**
   ```bash
   halfremembered-launcher server --otlp-endpoint http://localhost:9999
   ```
   Should show warning but continue running.

## Log Format

OTLP logs use the OpenTelemetry Logs API with structured attributes:

```rust
LogRecord {
    timestamp: SystemTime::now(),
    severity: Severity::Info,  // or Warn for stderr
    body: "log message text",
    attributes: [
        ("request_id", "unique-request-id"),
        ("client.hostname", "client-name"),
        ("process.binary", "/path/to/binary"),
        ("stream.type", "stdout"),  // or "stderr"
    ],
}
```

## Next Steps

When process streaming is implemented, logs from client processes (like `sregame.exe`) will be captured and forwarded in real-time to the configured OTLP endpoint.
