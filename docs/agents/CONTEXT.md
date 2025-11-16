# Context - HalfRemembered Launcher Project Bridge

## Project Mission
Building a secure SSH-based RPC system that enables server-initiated push operations to clients behind NAT/dynamic IPs. Inspired by OpenSSH's ControlMaster architecture, designed for game launcher use cases where servers need to push files and execute programs on player machines.

## Current Phase
**Production-Ready Core**: The launcher is operational with SSH-based file sync and execution. Recent work focuses on reliability improvements (large file transfers, connection stability) and workflow enhancements (auto-execute after sync, file watcher improvements).

**Memory System Bootstrap**: Just imported agent memory system and jj guidance from otlp-mcp project to enable better multi-model collaboration.

## Architecture Decisions

### Core Design
- **SSH-Only**: All communication over SSH - no custom protocols, no open ports
- **Persistent Connections**: Single SSH connection per client with heartbeats
- **Multiplexed Channels**: Control, data, and exec channels over one connection
- **Client-Initiated**: Clients connect to server (NAT/firewall traversal)
- **User Space**: Runs in user context with full environment access (no services)

### Protocol Design
- **Transport**: russh async SSH library
- **Authentication**: ssh-agent only (no password/key storage)
- **Framing**: Length-prefixed binary (4 bytes BE length + 1 byte type + payload)
- **Serialization**: bincode for efficiency (~3x smaller than JSON)
- **Max Message**: 10 MB (configurable)
- **Async Runtime**: Full tokio async/await

### File Transfer Strategy
- **SFTP Planned**: Currently using direct transfer, SFTP coming
- **Atomic Writes**: Write to temp, then rename
- **Checksums**: SHA256 verification
- **Change Detection**: File watcher for auto-execute triggers

## Work Completed

### Infrastructure (Operational)
- ✅ SSH-based client/server daemon architecture
- ✅ Protocol definition with bincode serialization
- ✅ Persistent connection management with heartbeats
- ✅ Reconnection logic with exponential backoff
- ✅ File sync with checksum verification
- ✅ Command execution in user context
- ✅ Client registry (online/offline status)
- ✅ Local CLI for server administration

### Recent Enhancements (Earlier)
- ✅ Auto-execute wired up to file watcher
- ✅ Post-sync execution support foundation
- ✅ Large file transfer failures resolved
- ✅ Client disconnect issues fixed
- ✅ File creation detection for cargo hardlinks
- ✅ Self-deployment configuration (.hrlauncher.toml)

### Critical Bug Fixes (2025-11-16 Session)
- ✅ Fix destination path construction (981c232) - Files synced to wrong locations
- ✅ Strip pattern base from paths (f540e67) - Assets going to `assets/assets/`
- ✅ Expand tilde in client paths (990bcbb) - Literal `~` directory created

### OTLP Integration Foundation (2025-11-16 Session)
- ✅ Added OpenTelemetry dependencies (c700d3c)
- ✅ Created otlp_exporter module with structured logging
- ✅ Added --otlp-endpoint CLI flag to server
- ✅ Server sends test log on initialization
- ✅ Created test-otlp.sh for quick testing iteration
- ✅ Documented workflow in OTLP_TESTING.md

### Documentation and Collaboration
- ✅ Imported jj guidance from otlp-mcp
- ✅ Added agent memory system to CLAUDE.md
- ✅ Created docs/agents/ memory system
- ✅ Updated NOW/PATTERNS/CONTEXT with session progress

## Key Discoveries

### Technical Insights
- Cargo hardlinks require special file watcher handling
- Large file transfers need proper chunking and flow control
- Client disconnects often stem from timeout mismatches
- Auto-execute timing is critical - must wait for file stability
- Binary serialization (bincode) provides significant bandwidth savings

### Workflow Insights
- Self-deployment scenario works (server can update itself)
- File watcher enables zero-touch deployment workflows
- User context execution essential for desktop app launching
- SSH-only approach eliminates firewall/NAT configuration pain

## Active Questions

### Technical
- Should we add progress reporting for large file transfers?
- How to handle partial file transfers on connection loss?
- How to efficiently stream process stdout/stderr over SSH channels?
- Should we batch OTLP log records or send individually?

### OTLP Implementation (Next Phase)
- Channel multiplexing: Reuse existing pattern from rsync channels
- Line buffering: Send complete lines or stream bytes?
- Backward compatibility: Keep ExecComplete for old clients?
- Error handling: What if OTLP endpoint becomes unreachable mid-stream?
- What's the optimal heartbeat interval for various network types?
- Should we implement command queuing for offline clients?

### Architecture
- When to migrate to SFTP for file transfers?
- How to handle client version mismatches?
- Should we add compression for large files?
- How to support multiple servers per client?

## Known Limitations
- No SFTP yet (planned future enhancement)
- File transfer is in-memory (not streaming)
- No persistence of offline command queue
- No metrics/telemetry yet
- Limited error recovery for partial transfers

## Handoff Ready

### For Development Work
Key files:
- `src/protocol.rs` - Message types and framing
- `src/client.rs` - Client daemon and control loop
- `src/server.rs` - Server daemon and client registry
- `src/execute.rs` - Execution handling and auto-execute
- `CLAUDE.md` - Complete agent guidance

### For Operations Work
Key configs:
- `.hrlauncher.toml` - Deployment configuration
- Default port: 20222
- Heartbeat: 30s
- Reconnect: 5s with exponential backoff

### For Memory System Work
Key files:
- `docs/agents/NOW.md` - Current state
- `docs/agents/PATTERNS.md` - Discovered patterns
- `docs/agents/CONTEXT.md` - This file
- `docs/agents/MEMORY_PROTOCOL.md` - Memory system guide

## Session History
- 2025-11-09: Imported memory system and jj guidance from otlp-mcp
- 2025-11-09: Auto-execute wired up to file watcher
- 2025-11-09: Post-sync execution support added
- 2025-11-09: Large file transfer and disconnect issues resolved

## Next Session Should Consider
1. **SFTP Migration**: Evaluate russh SFTP channel implementation
2. **Metrics**: Add basic telemetry for transfer sizes, timings
3. **Persistence**: Consider offline command queue persistence
4. **Testing**: Integration tests for file watcher + auto-execute
5. **Documentation**: User guide for deployment scenarios

## Technology Stack
- **Language**: Rust (stable)
- **Async Runtime**: tokio
- **SSH**: russh (async)
- **Serialization**: bincode + serde
- **Error Handling**: anyhow
- **File Watching**: notify (or similar)
- **Version Control**: git + jj (colocated)

## The HalfRemembered Philosophy
The name "HalfRemembered" reflects the architecture: servers don't need to remember client addresses (half-remembered) because clients maintain the connection. This enables operation with dynamic IPs, NAT, and restrictive firewalls - the client "remembers" how to reach the server, and the server uses that connection to push updates.
