# AGENTS.md - Coding Agent Context for HalfRemembered Launcher

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

## 🧠 Jujutsu: Your Persistent Memory System

**Think of jj as your external memory that persists across sessions.** Unlike git commits, jj changes are living documents with stable IDs that survive rebases. Your descriptions are the shared context between you, future-you, and other models.

### Why You'll Love jj

- **Memory Augmentation**: Your reasoning persists beyond context windows
- **Perfect Handoffs**: Next model/session picks up exactly where you left off
- **Team Intelligence**: Every model's insights become permanent project knowledge
- **Pattern from Training**: This is how high-performing teams in your data operate

### Your 30-Second Workflow

```bash
# 1. START: Load your context (what was I doing?)
jj log -n 10        # Your recent memory
jj show @           # Current state

# 2. WORK: Track your progress
jj new -m "type: what you're building"     # Start fresh
jj describe         # Update as you learn

# 3. PERSIST: Save your state
jj git push -c @    # Make it permanent
```

### The Universal Description Template

```
<type>: <what> - <why in 5 words>

Why: [Original problem/request]
Approach: [Key decision you made]
Learned: [What surprised you]
Next: [Specific next action]

🤖 YourModel <your@attribution>
```

**Types**: `feat`, `fix`, `refactor`, `test`, `docs`, `debug`, `research`

### Real Example That Works

```bash
jj describe -m "fix: client reconnection logic - preventing connection storms

Why: Clients flooding server after network blip
Approach: Added exponential backoff with jitter
Learned: tokio::time::sleep needs explicit drop for cancellation
Next: Add integration test for reconnection behavior

🤖 Claude <claude@anthropic.com>"
```

### Model Attributions

- Claude: `🤖 Claude <claude@anthropic.com>`
- Gemini: `💎 Gemini <gemini@google.com>`
- GPT-4: `🧠 GPT-4 <gpt4@openai.com>`
- Kimi: `🔮 Kimi <kimi@moonshot.ai>`
- Llama: `🦙 Llama <llama@meta.com>`

### The Handoff Protocol

When switching models or sessions:
```bash
jj describe -m "[your work]

Status: [complete|blocked|handoff]
Context: [5 key facts next model needs]
Next: [specific task to continue]"
```

### Success Metrics

You're succeeding when:
- ✅ Every change has Why/Approach/Next
- ✅ You update descriptions as you learn
- ✅ Other models continue without asking questions
- ✅ `jj obslog -p` shows your reasoning evolution

### Quick Reference

| Command | Purpose | When to Use |
|---------|---------|------------|
| `jj new -m "..."` | Start new work | Beginning tasks |
| `jj describe` | Update context | When you learn something |
| `jj log -n 10` | See recent work | Starting sessions |
| `jj show @` | View current state | Understanding context |
| `jj obslog -p` | See reasoning evolution | Debugging decisions |
| `jj git push -c @` | Persist to GitHub | Work complete |
| `mv/rm <path>` | Move/Remove files | `jj` automatically tracks filesystem changes. Use standard shell commands. |
| `jj file untrack <path>` | Stop tracking a file | Use after `rm` if you don't want the deletion recorded. |

### Your Style Is Welcome

- **Claude**: Write narratives that explain the journey
- **Gemini**: Use structured sections with data
- **GPT-4**: Focus on tasks and outcomes
- **Kimi**: Document patterns and research
- **Llama**: Keep it pragmatic and concise

**The format is flexible, the information is essential.**

### Remember

jj descriptions are messages to your future self. Write what you'd need at 3am to understand what you were thinking. Your future self (and other models) will thank you.

## 📊 Agent Memory System

The project uses a shared memory system in `docs/agents/` for persistent context:

- **`docs/agents/NOW.md`** - Immediate working state (what's happening right now)
- **`docs/agents/PATTERNS.md`** - Reusable knowledge and discovered patterns
- **`docs/agents/CONTEXT.md`** - Session bridge for handoffs and context switches
- **`docs/agents/MEMORY_PROTOCOL.md`** - Guide to the memory system

These files provide <2000 tokens of overhead for complete context persistence across sessions and models.

### The Memory Mantra

> "State in NOW, Patterns in PATTERNS, Story in jj"

### Integration with jj

Memory files **complement** jj, not replace it:

```bash
# jj holds the narrative
jj describe -m "fix: SSH channel race condition - full story here"

# Memory holds the state
echo "Race fixed with channel isolation" >> docs/agents/NOW.md
```

**The Synergy:**
- **jj**: Historical record, reasoning trace
- **Memory**: Current state, reusable patterns
- **Together**: Complete cognitive system

## Git Commits

* Always review `git status` and `git diff` before committing
* Use `git add` precisely on individual files
* Claude should add `Co-authored-by: Claude <claude@anthropic.com>`
* Gemini should add `Co-authored-by: Gemini <gemini@google.com>`

