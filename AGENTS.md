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
- **Async Runtime**: Full tokio async/await on both client and server
- **File transfer**: rsync delta algorithm over a dedicated SSH channel, NOT SFTP.
  SFTP appears once, in `ssh_client::upload_file_via_sftp`, used only by the `push`
  subcommand to place the binary itself on a remote host.

## Message Flow

The authoritative list is `protocol/src/lib.rs`. If this section disagrees with it,
this section is wrong.

### Client → Server (`ClientMessage`)
1. `Register`: Announce hostname and capabilities
2. `Heartbeat`: Keep-alive with timestamp
3. `RsyncComplete`: Report the outcome of a file transfer
4. `ExecComplete`: Report execution result
5. `Status`: Current client state
6. `Error`: Report an error to the server

### Server → Client (`ServerMessage`)
1. `Welcome`: Acknowledge registration and provide session info
2. `RsyncStart`: Begin a delta transfer for a path
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
7. `WatchDirectory`: Add a directory to the server's auto-sync watches.
8. `UnwatchDirectory`: Remove a watch.
9. `ListWatches`: List active watches.

### Server → CLI (`LocalResponse`)
1. `Success`: Acknowledge a successful command.
2. `Error`: Report an error in command execution.
3. `Status`: Provide server status information.
4. `ClientList`: Provide a list of connected clients.
5. `WatchList`: Provide the active watch list.

## Implementation Guidelines

### Concurrency
- Async runtime (tokio) for server-side client handling
- Separate threads for control loop and file transfers
- Non-blocking operations where possible
- Use channels (mpsc) for inter-thread communication

### Security

Implemented:
- SSH agent authentication only (no password/key storage)
- Checksum verification on every transfer, and again on a stored version before
  a rollback installs it

NOT implemented — do not read these as descriptions of the code:
- **Rate limiting on control messages.** There is none. Nothing in the tree
  throttles anything.
- **Path traversal validation.** `client_daemon::handle_rsync_start` tilde-expands
  the server-supplied `relative_path` and joins it to the working directory. There
  is no `..` check and no `canonicalize`; `Path::join` does not normalise.

**The invariant that makes the second one acceptable today, stated because
nothing in the code states it:** *the server is trusted.* A client accepts paths
and commands from its server because the connection is SSH-authenticated against
a known key, so a hostile path implies an already-compromised server — at which
point it can also just send an `Execute`. The traversal check is defence in depth,
not the thing holding the door.

That premise is load-bearing and it is worth knowing what would break it. Anyone
adding a second server, a shared build box, a relay, or any path where the sending
side is less trusted than the receiving side **must** add real path validation
first. This tool's trajectory is fleet distribution, which is exactly the direction
that erodes the assumption.

### File Transfer
- rsync delta algorithm — transfer only the changed parts of a file
- Atomic install: temp file in the destination directory, permissions set before
  the rename, fsync of both file and directory
- Executables carry deploy history and can be rolled back locally

## Install and Rollback

Two modules own what happens after the bytes arrive. Read their module docs
before changing either — each exists because of a specific failure.

- **`atomic_install.rs`** — tmp-fsync-rename. The destination is never a partial
  file, never briefly absent, and never briefly non-executable. This subsumes the
  older ETXTBSY unlink dance, which had a window where the path had no file at all.
- **`versioned_install.rs`** — content-addressed `.<name>.hrl-versions/` sidecar
  beside the destination, holding a deploy manifest and blobs. The destination
  stays an ordinary file, deliberately not a symlink. Rollback re-verifies a stored
  version against its checksum and refuses a corrupt one rather than installing it.
**Routing**: `versioned_install::should_version(dest, mode)` decides. The executable
bit decides for a destination we have never seen; a destination that already has
history keeps it regardless of incoming mode. Falling *into* versioning costs a few
KB; falling *out* of it silently costs the recovery path on a machine we may not be
able to reach again.

**Deliberately not done: no glibc preflight.** A `glibc_preflight` module existed
briefly and was removed on 2026-08-12. It scanned an incoming binary for `GLIBC_`
symbol tags and refused to activate one the target's glibc was too old to run.

It was removed because it was preemptive. Its own commit said so: build box and
target measured identical glibc, so the argument was trajectory rather than an
observed failure. Amy's call — *"I'm ok with handling the occasional failure."*

If you are considering adding it back, know what you are trading. The reasoning
was sound: glibc symbol versioning is one-directional, so a build box on a rolling
distro drifts ahead of an appliance target by default, and the failure lands at
service start looking like a bad build. But it never fired, it cost ~143 lines of
non-test code, it was blind to packed executables (the tags hide inside the
compressed payload, so it silently passed them), and it read the local version by
scraping `ldd --version` output on the target. Rollback already covers the failure
it was preventing, and it covers it whatever the cause.

The removed code is in git history at `f58bf0b`.

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

## Persistent Context Across Sessions

**This project uses git. Jujutsu is not installed, here or anywhere on this fleet.**
An earlier version of this document described jj as your memory system at length;
that guidance was removed on 2026-08-12 along with the repo's stale `.jj/` directory
(archived to `~/archive/halfremembered-launcher/`). If you find jj instructions in a
sibling repo, they are stale too.

What actually carries context between sessions:

- **git history** — the narrative. Write commit messages for the person who arrives
  at 3am with no context: what broke, what you decided, what surprised you. A commit
  message is the cheapest durable memory in the project, and the only one that
  travels with the code.
- **`docs/agents/`** — durable project truth, verifiable against the code beside it.
- **`signoff.md`** at the repo root — ephemeral session handoff, gitignored, never
  committed. What is mid-flight, what is blocked, what was decided today. Refresh it
  before winding down; it is minutes for you against a very expensive cold read for
  whoever is next.

### Verify, Don't Relay

A claim in a document is not evidence about the code. Neither is a summary from
another session, however confident. Both are worth reading and neither is worth
trusting on its own — this file has been wrong about the wire protocol, and a
reported test count was wrong for a whole day before someone re-ran the suite.

Read the source. Run the tests. When a doc and the code disagree, the code wins and
the doc gets fixed in the same change.

## 📊 Agent Memory System

The project uses a shared memory system in `docs/agents/` for persistent context:

- **`docs/agents/NOW.md`** - Immediate working state (what's happening right now)
- **`docs/agents/PATTERNS.md`** - Reusable knowledge and discovered patterns
- **`docs/agents/CONTEXT.md`** - Session bridge for handoffs and context switches
- **`docs/agents/MEMORY_PROTOCOL.md`** - Guide to the memory system

These files provide <2000 tokens of overhead for complete context persistence across sessions and models.

### The Memory Mantra

> "State in NOW, Patterns in PATTERNS, Story in git, Handoff in signoff.md"

The split is about lifetime, not format. Git holds why a change happened and keeps
it forever. The memory files hold what is true right now and get rewritten as that
changes. `signoff.md` holds what only matters until the next session picks it up.

Putting session state in git leaves a permanent record of something that stopped
being true; putting durable truth only in `signoff.md` loses it the moment the file
is refreshed.

## Git Commits

* Always review `git status` and `git diff` before committing
* Use `git add` precisely on individual files
* Claude should add `Co-authored-by: Claude <claude@anthropic.com>`
* Gemini should add `Co-authored-by: Gemini <gemini@google.com>`

