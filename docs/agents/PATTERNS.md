# Patterns - HalfRemembered Launcher Project

## SSH and Network Patterns

### Pattern: SSH-Only Security Model
WHEN: Building remote control systems
USE: All communication over SSH, no custom protocols
WHY: Leverages battle-tested SSH security, no open ports needed
BENEFIT: NAT/firewall traversal, built-in authentication
GOTCHA: Requires ssh-agent setup on all clients
APPLIES: Core architecture decision for this project

### Pattern: Persistent SSH with Multiplexing
WHEN: Need multiple operation types over one connection
USE: Single SSH session with multiple channels (control, data, exec)
WHY: Reduces connection overhead, maintains state
BENEFIT: ControlMaster-like architecture, efficient resource usage
GOTCHA: Channel management complexity, need proper cleanup
CODE: See src/protocol.rs for channel types

### Pattern: Client-Initiated Outbound Connections
WHEN: Clients behind NAT/dynamic IPs
USE: Clients connect to server, server pushes over established connection
WHY: Eliminates need for port forwarding, works with dynamic IPs
BENEFIT: Works anywhere, client has full user context
GOTCHA: Server must maintain client registry
APPLIES: Core to the "half-remembered" concept

## Concurrency and Async Patterns (Rust + Tokio)

### Pattern: Tokio Async Runtime for I/O
WHEN: Network operations with multiple concurrent clients
USE: tokio async/await for SSH connections and file operations
WHY: Efficient handling of many concurrent connections
BENEFIT: Scales well, non-blocking I/O
GOTCHA: Must handle cancellation properly with tokio::select!
CODE: Throughout src/client.rs and src/server.rs

### Pattern: MPSC Channels for Control Flow
WHEN: Need to coordinate between async tasks
USE: tokio::sync::mpsc for control messages
WHY: Type-safe message passing between tasks
BENEFIT: Clean separation of concerns
GOTCHA: Need to handle channel closure gracefully
APPLIES: Client/server control loops

## File Transfer and Sync Patterns

### Pattern: Atomic File Write (Temp + Rename)
WHEN: Transferring files that may be in use
USE: Write to .tmp file, then atomic rename
WHY: Prevents partial/corrupted files
BENEFIT: Crash-safe, no race conditions
GOTCHA: Need proper cleanup of temp files on failure
CODE: See file transfer implementation

### Pattern: Checksum Verification
WHEN: Transferring files over network
USE: SHA256 checksums before and after transfer
WHY: Ensures data integrity
BENEFIT: Detects corruption, validates transfer
GOTCHA: CPU overhead for large files
APPLIES: All file sync operations

### Pattern: File Watcher for Change Detection
WHEN: Need to detect file updates for auto-execute
USE: File system watcher with event filtering
WHY: Immediate response to file changes
BENEFIT: Enables workflow automation
GOTCHA: Cargo hardlinks need special handling
LEARNED: File creation events essential for cargo output

## Protocol and Message Patterns

### Pattern: Length-Prefixed Binary Framing
WHEN: Need efficient, type-safe network protocol
USE: [4 bytes BE length][1 byte type][N bytes bincode payload]
WHY: Simple, efficient, self-describing
BENEFIT: Fixed parsing overhead, no delimiter escaping
GOTCHA: Need max message size protection (10 MB default)
CODE: See src/protocol.rs

### Pattern: bincode for Serialization
WHEN: Rust-to-Rust communication
USE: bincode with serde
WHY: Fast, compact (~3x smaller than JSON), type-safe
BENEFIT: Zero-copy deserialization, strong typing
GOTCHA: Not human-readable, versioning needs care
APPLIES: All protocol messages

### Pattern: Message Type Enum
WHEN: Multiple message types in protocol
USE: Rust enum with serde serialization
WHY: Type safety, exhaustive matching
BENEFIT: Compile-time correctness, easy to extend
GOTCHA: Breaking changes require version migration
CODE: Register, Heartbeat, SyncFile, Execute, etc.

## Reliability Patterns

### Pattern: Exponential Backoff with Jitter
WHEN: Automatic reconnection after network failure
USE: Exponential backoff (5s → 10s → 20s) with random jitter
WHY: Prevents connection storms, gives network time to recover
BENEFIT: Self-healing system, reduces server load
GOTCHA: Need max backoff limit
APPLIES: Client reconnection logic

### Pattern: Heartbeat Keep-Alive
WHEN: Need to detect dead connections
USE: Periodic heartbeat messages (default 30s)
WHY: Detects network failures, keeps connection alive
BENEFIT: Fast failure detection, prevents firewall timeouts
GOTCHA: Need timeout tuning for different networks
CODE: Client heartbeat loop

### Pattern: Graceful Shutdown with Timeout
WHEN: Need to cleanly stop daemon
USE: Shutdown message with timeout, then force close
WHY: Allows in-flight operations to complete
BENEFIT: No data loss, clean resource cleanup
GOTCHA: Need reasonable timeout values
APPLIES: Both client and server shutdown

## Execution Patterns

### Pattern: Post-Sync Auto-Execute
WHEN: Files synced and ready to use
USE: File watcher triggers execution after sync complete
WHY: Enables deployment workflows, automatic updates
BENEFIT: Zero-touch deployments
GOTCHA: Need proper file permissions, path handling
RECENT: Just implemented in latest commits

### Pattern: User Context Execution
WHEN: Running programs on client
USE: Execute in user space with full environment
WHY: Access to display, audio, user files, credentials
BENEFIT: No privilege escalation needed, full desktop access
GOTCHA: Security implications of running arbitrary code
APPLIES: Core feature for game launcher use case

## Error Handling Patterns (Rust Specific)

### Pattern: anyhow::Result for Fallible Operations
WHEN: Any operation that can fail
USE: anyhow::Result<T> with context chaining
WHY: Rich error context, easy error propagation
BENEFIT: Excellent debugging info, composable
GOTCHA: Never use unwrap() - always propagate with ?
APPLIES: All fallible operations in codebase

### Pattern: Context Addition on Errors
WHEN: Propagating errors up the stack
USE: .context("what operation failed") on results
WHY: Builds error chain with operation context
BENEFIT: Pinpoints exact failure location
EXAMPLE: `.context("failed to sync file to client")?`
APPLIES: All error paths

## Memory System Patterns (Just Added)

### Pattern: Three-Layer Memory (NOW/PATTERNS/CONTEXT)
WHEN: Need persistent context across sessions
USE: NOW.md (immediate) + PATTERNS.md (permanent) + CONTEXT.md (bridge)
WHY: Optimizes token usage while preserving knowledge
BENEFIT: <2000 tokens overhead for complete context
GOTCHA: Must maintain actively or becomes stale
APPLIES: Multi-model collaboration

### Pattern: jj for Living Documentation
WHEN: Tracking changes and reasoning
USE: jj changes with rich descriptions (Why/Approach/Learned/Next)
WHY: Context persists beyond git commits, supports rebasing
BENEFIT: Perfect handoffs between models/sessions
GOTCHA: Requires discipline in description writing
STATUS: Just enabled in this repo

## Path Handling Patterns (Added 2025-11-16)

### Pattern: Strip Glob Base from Relative Paths
WHEN: Using glob patterns like `assets/**/*` in sync rules
USE: Extract non-glob prefix and strip from matched paths before joining
WHY: Prevents duplication like `~/dest/assets/` + `assets/data/file` → `~/dest/assets/assets/data/file`
BENEFIT: Files sync to correct locations
EXAMPLE: Pattern `assets/**/*` → strip `assets/` → join with destination
CODE: `SshServer::strip_pattern_base()` in ssh_server.rs
DISCOVERED: Files were going to `assets/assets/` instead of `assets/`

### Pattern: Tilde Expansion on Client
WHEN: Paths from server contain `~` for home directory
USE: `expand_tilde()` helper before using paths
WHY: Client needs absolute paths, `~` is shell-specific
BENEFIT: Cross-platform compatibility (Unix/Windows)
GOTCHA: Don't expand on server side - let client determine its own home
CODE: `expand_tilde()` in client_daemon.rs
APPLIES: File paths, working directories, execute commands

### Pattern: Destination Path = Rule Destination + Stripped Relative Path
WHEN: Constructing client-side file paths from sync rules
USE: `rule.destination.join(strip_pattern_base(pattern, relative_path))`
WHY: Gives flexibility in organizing client-side file layout
BENEFIT: Source and destination can have different structures
EXAMPLE: Server `target/release/bin` → Client `~/bin/`
APPLIES: File watcher callbacks, initial sync

## OpenTelemetry Integration Patterns (Added 2025-11-16)

### Pattern: Optional OTLP via CLI Flag
WHEN: Adding observability without breaking existing workflows
USE: `--otlp-endpoint <URL>` flag that only initializes if provided
WHY: Zero overhead when not needed, opt-in behavior
BENEFIT: Graceful degradation if endpoint unreachable
GOTCHA: Must handle initialization failures gracefully
CODE: `ssh_server.rs::run()` with otlp_endpoint parameter
APPLIES: Server startup, future metrics/tracing

### Pattern: Structured Logs with Attributes
WHEN: Sending logs to OTLP collector
USE: OpenTelemetry LogRecord with key-value attributes
WHY: Enables rich querying and filtering in observability tools
BENEFIT: Request correlation, multi-dimensional analysis
EXAMPLE: `request_id`, `client.hostname`, `process.binary`, `stream.type`
CODE: `OtlpExporter::send_log()` in otlp_exporter.rs
APPLIES: Process stdout/stderr forwarding (in progress)

### Pattern: Test Scripts for Quick Iteration
WHEN: Developing features requiring external services
USE: Shell script that starts system with dependencies
WHY: Reduces friction, makes testing repeatable
BENEFIT: Anyone can test with one command
EXAMPLE: `./test-otlp.sh` starts server with OTLP enabled
APPLIES: Integration testing, manual verification

## Protocol Patterns (Existing but Highlighted)

### Pattern: Message Types Already Defined
WHEN: Adding new protocol features
USE: Check `protocol/src/message_types.rs` first
WHY: Avoid duplicating message type constants
BENEFIT: Protocol consistency, forward compatibility
EXAMPLE: `MSG_EXEC_STDOUT`, `MSG_EXEC_STDERR` already exist!
DISCOVERED: Exec streaming types were pre-defined, just unused
APPLIES: Future protocol extensions

### Pattern: Gradual Protocol Evolution
WHEN: Adding new message types to existing protocol
USE: Add new types without removing old ones
WHY: Backward compatibility with older clients
BENEFIT: Rolling upgrades, no flag day
EXAMPLE: Keep `ExecComplete` even when adding streaming
APPLIES: All protocol changes
