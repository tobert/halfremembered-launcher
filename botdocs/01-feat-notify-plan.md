# Feature: Filesystem Watching with Auto-Sync

**Created:** 2025-10-14
**Author:** Claude (Anthropic)
**Status:** Planning
**Target:** HalfRemembered Launcher v0.1.x

## Overview

Add filesystem watching capability to the HalfRemembered Launcher server, enabling automatic replication of file changes to connected clients. When files change in watched directories, the server will automatically push those changes to all connected clients via the existing rsync protocol.

## Context

The HalfRemembered Launcher is an SSH-based RPC system that maintains persistent connections from clients to a server. Currently, file syncing is manual (via CLI `sync` command). This feature adds automatic syncing triggered by filesystem events, enabling development workflows like "edit on workstation, auto-deploy to test clients."

### Use Cases
- **Development**: Edit code on main machine, automatically sync to remote test machines
- **Configuration Management**: Update config files, auto-replicate to fleet
- **Asset Distribution**: Drop files in watched directory, auto-distribute to clients

## Dependencies

### New Crate Dependencies
Add to `Cargo.toml` workspace dependencies:

```toml
notify = "8.2.0"              # Cross-platform filesystem notification
notify-debouncer-mini = "0.5" # Event debouncing (simple, adequate)
globset = "0.4"               # Pattern matching (used by ripgrep)
```

**Platform Support:**
- Linux: inotify
- Windows: ReadDirectoryChangesW
- macOS: FSEvents
- FreeBSD/BSD: kqueue
- Fallback: polling

All handled automatically by `notify` crate, no platform-specific code needed.

## Architecture

### Protocol Changes

**File:** `protocol/src/lib.rs`

Add new `LocalCommand` variants:
```rust
WatchDirectory {
    path: String,
    recursive: bool,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}
UnwatchDirectory {
    path: String,
}
ListWatches
```

Add new types:
```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WatchInfo {
    pub path: String,
    pub recursive: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}
```

Add new `LocalResponse` variant:
```rust
WatchList {
    watches: Vec<WatchInfo>,
}
```

### File Watcher Component

**File:** `launcher/src/file_watcher.rs` (new file)

Core responsibilities:
- Manage `notify-debouncer-mini` watcher instance
- Store per-watch configuration: `(absolute_path, recursive_flag, include_globset, exclude_globset)`
- Handle filesystem events (Create, Modify)
- Apply include/exclude filters using `globset`
- Trigger `sync_file_to_clients` for matching files
- Compute relative paths from watch root

Key implementation details:
- Use `RecursiveMode::Recursive` or `NonRecursive` based on watch config
- 100ms debounce window (notify-debouncer-mini default)
- Only process regular files (ignore directories, symlinks, special files)
- Pattern matching against relative path from watch root

### Server State Updates

**File:** `launcher/src/ssh_server.rs`

Add fields to `SshServer`:
```rust
pub struct SshServer {
    // ... existing fields ...
    watched_dirs: Arc<Mutex<HashMap<PathBuf, WatchConfig>>>,
    file_watcher: Arc<Mutex<Option<FileWatcher>>>,
}

struct WatchConfig {
    recursive: bool,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}
```

Update `handle_local_command` to handle:
- `WatchDirectory`: Canonicalize path, compile glob patterns, add to watcher, log config
- `UnwatchDirectory`: Remove from watcher, clean up state
- `ListWatches`: Return current watch configurations

### CLI Updates

**File:** `launcher/src/main.rs`

Add new subcommands:
```bash
# Add watch with filtering
halfremembered-launcher watch /path/to/dir \
  --recursive \
  --include '*.rs' --include '*.toml' \
  --exclude '*.o' --exclude 'target/*' --exclude '.git/*'

# Remove watch
halfremembered-launcher unwatch /path/to/dir

# List all watches
halfremembered-launcher list-watches
```

Command structure:
```rust
Watch {
    directory: PathBuf,
    #[arg(long, default_value = "true")]
    recursive: bool,
    #[arg(long)]
    include: Vec<String>,
    #[arg(long)]
    exclude: Vec<String>,
}
Unwatch {
    directory: PathBuf,
}
ListWatches
```

## Path Handling Strategy

### Core Decision: Use Only `std::path::PathBuf`

**No additional path handling library needed.** Use Rust's standard library `std::path::PathBuf` for all path operations. It already handles Windows vs Unix paths correctly, and the relative-path-over-the-wire strategy eliminates the need for cross-platform path translation.

### Key Principle: Relative Paths Over The Wire

**Server side:**
1. Accept path from user (can be relative like `./src` or absolute like `/home/user/project/src`)
2. Canonicalize to absolute path: `path.canonicalize()`
3. Store absolute path internally in server's native filesystem format
4. On file event: compute `relative_path = changed_file.strip_prefix(watch_root)`
5. Send **only the relative path** to clients via `ServerMessage::RsyncStart`

**Client side:**
1. Receive relative path (e.g., `src/main.rs`)
2. Write file to relative location from client's current directory
3. Client resolves in its own native filesystem format

**Why this works:**
- Server operates in its own filesystem space (Linux paths, Windows paths, WSL paths, etc.)
- Clients operate in their own filesystem space (could be different OS than server)
- Only relative paths cross the wire - no translation needed
- Each side uses native `PathBuf` operations in its own context

### Example Flow

```
User input: watch ./myproject --recursive
Server (Linux): canonicalize to /home/user/myproject
File changes: /home/user/myproject/src/lib.rs
Compute relative: src/lib.rs
Send to clients: RsyncStart { relative_path: "src/lib.rs", ... }

Client 1 (Windows): receives "src/lib.rs" → writes to .\src\lib.rs
Client 2 (Linux): receives "src/lib.rs" → writes to ./src/lib.rs
Client 3 (macOS): receives "src/lib.rs" → writes to ./src/lib.rs
```

### WSL Interop

**Works automatically** because each side canonicalizes in its native filesystem space:
- Server on WSL: watches `/mnt/c/Users/alice/project`, sends relative path `main.rs`
- Server on Windows: watches `C:\Users\alice\project`, sends relative path `main.rs`
- Client on WSL: writes to `./main.rs` (resolves to `/home/user/main.rs`)
- Client on Windows: writes to `.\main.rs` (resolves to `C:\Users\bob\main.rs`)

WSL-specific paths (`/mnt/c/...` and `\\wsl$\...`) are just native paths on their respective platforms.

### Logging

Log both input path and canonicalized absolute path at **Info** level:
- Input: `"Watching src (resolved: /home/user/project/src, recursive: true, exclude: [*.o])"`
- Events: `"File changed: /home/user/project/src/main.rs (relative: src/main.rs)"`
- Client: `"Syncing src/main.rs (resolved: /home/client/src/main.rs)"`

## Pattern Matching Behavior

### Include Patterns (Optional)
- If specified: file must match **at least one** include pattern to be synced
- If not specified: all files are candidates for syncing
- Examples: `*.rs`, `src/**/*.toml`, `config.json`

### Exclude Patterns (Optional)
- File must **not match any** exclude pattern to be synced
- Applied after include patterns
- Examples: `*.o`, `*.tmp`, `.git/*`, `target/*`, `node_modules/*`

### Matching Details
- Patterns matched against **relative path** from watch root
- Uses `globset` crate (same as ripgrep) for multi-pattern matching
- Patterns compiled at watch creation time for efficiency
- Invalid patterns logged as errors, watch creation fails

### Common Examples
```bash
# Rust project: only source files, skip build artifacts
--include '*.rs' --include 'Cargo.toml' --include 'Cargo.lock' \
--exclude 'target/*'

# Config files only
--include '*.conf' --include '*.json' --include '*.yaml'

# Everything except common noise
--exclude '*.o' --exclude '*.tmp' --exclude '.git/*' \
--exclude 'node_modules/*' --exclude '__pycache__/*'
```

## Implementation Sequence

### TODO

- [ ] **Phase 1: Dependencies and Protocol**
  - [ ] Add `notify`, `notify-debouncer-mini`, `globset` to `Cargo.toml`
  - [ ] Add `WatchDirectory`, `UnwatchDirectory`, `ListWatches` to `LocalCommand`
  - [ ] Add `WatchInfo` struct and `WatchList` to `LocalResponse`
  - [ ] Run `cargo build` to verify dependencies

- [ ] **Phase 2: File Watcher Component**
  - [ ] Create `launcher/src/file_watcher.rs`
  - [ ] Implement `WatchConfig` struct (path, recursive, patterns)
  - [ ] Implement `FileWatcher` struct with `notify-debouncer-mini`
  - [ ] Add methods: `add_watch()`, `remove_watch()`, `list_watches()`
  - [ ] Implement event handler with filtering logic
  - [ ] Add logging for all watch operations (Info level)
  - [ ] Compile glob patterns with error handling

- [ ] **Phase 3: Server Integration**
  - [ ] Add `file_watcher` module to `launcher/src/lib.rs`
  - [ ] Add `watched_dirs` and `file_watcher` fields to `SshServer`
  - [ ] Implement `handle_local_command` cases for watch operations
  - [ ] Path canonicalization with logging
  - [ ] Wire up file watcher event handler to `sync_file_to_clients`

- [ ] **Phase 4: CLI Commands**
  - [ ] Add `Watch`, `Unwatch`, `ListWatches` subcommands to CLI
  - [ ] Add `--recursive`, `--include`, `--exclude` flags
  - [ ] Implement command handlers
  - [ ] Add help text with common examples
  - [ ] Test CLI argument parsing

- [ ] **Phase 5: Testing and Documentation**
  - [ ] Manual testing: watch directory, modify files, verify client sync
  - [ ] Test recursive vs non-recursive
  - [ ] Test include/exclude patterns
  - [ ] Test path canonicalization (relative input → absolute storage)
  - [ ] Test multiple watches on different directories
  - [ ] Update CLAUDE.md with file watcher architecture notes
  - [ ] Test on Linux (primary platform)
  - [ ] Test on Windows if available
  - [ ] Test WSL interop if available

- [ ] **Phase 6: Polish**
  - [ ] Review all log messages for clarity
  - [ ] Ensure consistent error messages
  - [ ] Code style review
  - [ ] Final commit with comprehensive message

## Design Decisions

### Simplicity Over Performance
- **100ms debounce**: Simple, adequate for most use cases
- **Broadcast to all clients**: No client filtering in v1
- **No persistence**: Watches cleared on server restart (acceptable for development use case)
- **Single-threaded event handling**: Adequate for moderate file change volumes

### Platform Portability
- Use `notify` crate's automatic platform detection
- No platform-specific code paths
- Standard Rust path handling (`std::path`)
- Let each platform use its native, most efficient backend

### Filter Design
- Glob patterns (familiar to users from shell, ripgrep, .gitignore)
- Include = allowlist (optional)
- Exclude = blocklist (optional)
- Relative path matching (intuitive)

### Scope Limitations (Not in v1)
- No per-client watch filtering
- No watch persistence across restarts
- No bandwidth throttling
- No conflict resolution for simultaneous edits
- No symlink following (security consideration)

## User Prompts (Conversation Evolution)

### Initial Request
> ok next feature: let's look for a good crate to do inotify watching, portable would be good but it's fine if it only works on Linux. When we're done we'll be able to instruct the server to watch a directory and replicate that directory's files to the clients when there are changes on the filesystem

### Refinement: Platform Support and Flags
> awesome. let's add a --recursive flag where appropriate and do so. if the notify library has sensible defaults let's use them. otherwise let's make sure to set up Linux, Windows, and MacOS, maybe FreeBSD if it's easy. I'm not worried about high volumes of events or high performance. adequate & simple & portable would be preferable. show me the plan again with these things in mind. CLI design looks good. Yes do path resolution to absolute paths as needed, make sure to log the input and actual path at Info level. On the other side, recompute the relative to absolute paths.

### Addition: Pattern Filtering
> one more thing, please include some flags to go with --recursive that filter paths in and out for e.g. avoiding .o files

### Final Request: Documentation
> good! I like it. Let's make sure to use a path handling library that can deal with Windows and unix-likes appropriately. There are some interesting options around Windows Subsystem for Linux where WSL can be addressed by a UNC path from Windows and Windows can be addressed under /mnt/c on Linux. Think about that a bit and then let's see this very good plan in a markdown file called botdocs/01-feat-notify-plan.md, complete with context and TODO. We will use this as our source of truth as we build this feature and update it as we go. include this prompt at the end of the doc, and credit yourself in the doc. The target audience is mostly Claude Code.

### Path Handling Review
> ok agreed on relative paths. review what we've said in the doc about paths and ensure it's aligned with that and using std::path::PathBuf. include this prompt in the doc as well

---

**Document Status:** This is a living document. Update TODO checkboxes as work progresses. Add notes about implementation challenges, decisions, or deviations from the plan in a "Implementation Notes" section below.

## Implementation Notes

_(Add notes here as implementation progresses)_
