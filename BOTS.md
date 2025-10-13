# BOTS.md - Coding Agent Context for HalfRemembered Launcher

HalfRemembered Launcher is a secure SSH-based RPC system inspired by OpenSSH's ControlMaster architecture. It enables server-initiated push operations to clients behind NAT/dynamic IPs using persistent SSH connections with multiplexed channels.

## Core Design Principles

1. **SSH-Only Security**: All network communication happens over SSH - no custom protocols, no open ports
2. **Persistent Connections**: Single SSH connection per client, kept alive with heartbeats
3. **Multiplexed Channels**: Control, data, and exec channels over one SSH connection
4. **User Context**: Client runs in user space with full environment access (3D, audio, etc.)
5. **Self-Contained**: No systemd/Windows services, runs entirely in user space

## Component Architecture

### Client Daemon
- Establishes outbound SSH connection to server
- Maintains persistent connection with auto-reconnect
- Runs control loop to handle server commands
   - Receives files from server and writes them to local filesystem
   - Executes programs in user context with user's permissions

### Server Daemon
- Accepts incoming SSH connections from clients
- Maintains client registry with online/offline status
- Pushes files in fanout to connected clients
- Pushes commands in fanout to connected clients
- Provides CLI for: file push, command execution, client listing

### Protocol
- **Transport**: Single SSH session per client (russh async)
- **Authentication**: ssh-agent only via russh's agent client
- **Framing**: `[4 bytes BE length][1 byte type][N bytes bincode payload]`
- **Serialization**: bincode (binary format, type-safe with serde, ~3x smaller than JSON)
- **Max message size**: 10 MB (configurable)
- **Message types**: Register, Heartbeat, SyncFile, Execute, Status, Ping, Shutdown
- **Async Runtime**: Full tokio async/await on both client and server
- **Future**: SFTP channel for file transfers (not yet implemented)

## Message Flow

### Client → Server
1. `Register`: Announce hostname and capabilities
2. `Heartbeat`: Keep-alive with timestamp
3. `FileReceived`: Acknowledge file transfer
4. `ExecComplete`: Report execution result
5. `Status`: Current client state
6. `Error`: Report an error to the server

### Server → Client
1. `Welcome`: Acknowledge registration and provide session info
2. `SyncFile`: Initiate file transfer with path and checksum
3. `Execute`: Run program with arguments
4. `Ping`: Request immediate heartbeat
5. `Shutdown`: Graceful disconnect

## Local CLI Protocol

The server daemon also listens for local commands from the CLI on the same SSH port. This is a separate protocol used for server administration.

### CLI → Server (`LocalCommand`)
1. `Status`: Request server status and connected client list.
2. `Ping`: Ping a specific connected client.
3. `ListClients`: Get a list of all connected clients.
4. `Shutdown`: Shut down the server daemon.
5. `SyncFile`: Request the server to sync a file to all clients.
6. `Execute`: Request the server to execute a command on a specific client.

### Server → CLI (`LocalResponse`)
1. `Success`: Acknowledge a successful command.
2. `Error`: Report an error in command execution.
3. `Status`: Provide server status information.
4. `ClientList`: Provide a list of connected clients.

## Implementation Guidelines

### Concurrency
- Async runtime (tokio) for server-side client handling
- Separate threads for control loop and file transfers
- Non-blocking operations where possible
- Use channels (mpsc) for inter-thread communication

### Security
- SSH agent authentication only (no password/key storage)
- Validate all paths to prevent directory traversal
- Checksum verification for file transfers
- Rate limiting on control messages

### File Transfer
- Use SFTP for reliability
- Transfer only changed files (mtime/size comparison)
- Atomic file operations (write to temp, then rename)

## Configuration

**Defaults** (all configurable via CLI):
- Server port: `20222`
- Heartbeat: `30` seconds
- Reconnect: `5` seconds with exponential backoff
- Max message: `10` MB

**Platform notes**:
- Linux: SSH agent via `SSH_AUTH_SOCK`, supports native Linux and WSL
- Windows: SSH agent via OpenSSH for Windows, ensure executable permissions after transfer
- WSL can launch .exe into Windows user context

## Development Guidelines

**Error Handling**:
- Use `anyhow::Result` for all fallible operations
- Never use `unwrap()` - always propagate errors with `?`
- Add context with `.context()` for debugging
- Never silently discard errors with `let _ =`
- Handle reconnection gracefully on network failures

**Code Style**:
- Prioritize correctness and clarity over performance
- No organizational comments that summarize code
- Comments should only explain "why" when non-obvious
- Implement functionality in existing files unless it's a new logical component
- Avoid `mod.rs` files - use `src/module_name.rs` directly
- Use full words for variable names (no abbreviations)

## Git Commits

* Always review `git status` and `git diff` before committing
* Use `git add` precisely on individual files
* Claude should add `Co-authored-by: Claude <claude@anthropic.com>`
* Gemini should add `Co-authored-by: Gemini <gemini@google.com>`

