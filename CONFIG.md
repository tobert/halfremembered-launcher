# Configuration File Guide

HalfRemembered Launcher supports project-based configuration via `.hrlauncher.toml` files. This enables automatic filesystem watching and syncing for development workflows.

## Quick Start

1. Create a `.hrlauncher.toml` in your project root:

```toml
[project]
name = "my-game"

[[sync]]
include = ["target/release/*.exe", "assets/**/*"]
destination = "games/my-game/"
```

2. Start config-based syncing:

```bash
halfremembered-launcher config-sync --server user@host
```

That's it! Files matching your patterns will automatically sync to connected clients when changed.

## File Location

The `.hrlauncher.toml` file should be placed in your project root directory (typically next to `Cargo.toml`, `package.json`, etc.).

The `config-sync` command searches for `.hrlauncher.toml` in:
1. Current directory
2. Parent directories (recursively up to filesystem root)

This lets you run `config-sync` from anywhere within your project tree.

## Configuration Structure

### Required Fields

```toml
[project]
name = "project-name"        # Required: Used in logs

[[sync]]                      # At least one sync rule required
include = ["pattern"]         # Required: What files to sync
destination = "path/"         # Required: Where to write on clients
```

### Optional Fields

```toml
[project]
description = "Optional project description"

[[sync]]
name = "rule-name"           # Optional: Name for logs
exclude = ["pattern"]        # Optional: Files to skip
clients = ["pattern"]        # Optional: Target specific clients (default: all)
mirror = false               # Optional: Delete files not in source (default: false)
```

## Sync Rules

Each `[[sync]]` block defines a set of files to watch and sync. You can have multiple sync rules for different file types or destinations.

### Include Patterns

Glob patterns specify which files to watch and sync:

```toml
# Single file
include = ["config.toml"]

# All files in directory (non-recursive)
include = ["src/*.rs"]

# Recursive matching with **
include = ["assets/**/*"]

# Multiple patterns
include = [
    "*.exe",
    "*.dll",
    "assets/**/*.png",
]
```

**Pattern Syntax:**
- `*` - Matches any string except `/`
- `**` - Matches any string including `/` (recursive)
- `?` - Matches any single character
- `[abc]` - Matches one character in the set
- `{a,b}` - Matches either pattern

### Exclude Patterns

Optional patterns to skip files matched by `include`:

```toml
[[sync]]
include = ["assets/**/*"]
exclude = [
    "**/*.psd",      # Source files
    "**/*.blend",
    "**/.git/**",    # Version control
    "**/*.tmp",      # Temporary files
]
```

Exclude patterns are applied **after** include patterns. A file must:
1. Match at least one `include` pattern
2. NOT match any `exclude` pattern

### Destination Paths

The `destination` field specifies where files are written on clients:

```toml
# Relative path (relative to client's working directory)
destination = "."
destination = "bin/"
destination = "games/mygame/"

# Home directory expansion
destination = "~/games/mygame/"

# Absolute path (use with caution - may differ across platforms)
destination = "/opt/games/mygame/"
```

**Path Resolution:**
- Patterns are resolved relative to `.hrlauncher.toml` location
- Destination paths are resolved on each client in their native format
- Example: Linux server syncing to Windows client works seamlessly

### Client Filtering

Target specific clients by hostname pattern:

```toml
# Sync only to Windows machines
clients = ["windows-*"]

# Multiple patterns
clients = ["dev-*", "test-*"]

# Sync to all clients (default)
# clients = []  # or omit the field
```

Client filtering uses glob patterns matched against client hostnames reported during connection registration.

### Mirror Mode

When `mirror = true`, files on the client that don't exist in the source will be **deleted**.

```toml
[[sync]]
include = ["assets/**/*"]
destination = "assets/"
mirror = true  # Delete assets on client that aren't in source
```

**Use with caution!** Mirror mode performs destructive operations. Best for:
- Asset directories that should exactly match source
- Configuration files that may be removed
- Build outputs that should be cleaned

**Not recommended for:**
- User data directories
- Locations with client-generated files
- Shared directories

## Example Configurations

### Bevy Game (Windows Cross-Compile from Linux)

```toml
[project]
name = "my-bevy-game"
description = "Cross-platform Bevy game development"

# Sync Windows executable and DLLs
[[sync]]
name = "windows-binaries"
include = [
    "target/x86_64-pc-windows-msvc/debug/*.exe",
    "target/x86_64-pc-windows-msvc/debug/*.dll",
]
destination = "games/mygame/"
clients = ["windows-*"]  # Only to Windows clients

# Sync assets to all clients
[[sync]]
name = "game-assets"
include = ["assets/**/*"]
exclude = ["**/*.psd", "**/*.blend", "**/.DS_Store"]
destination = "games/mygame/assets/"
mirror = true  # Keep asset directory clean
```

### Rust CLI Tool

```toml
[project]
name = "my-cli-tool"

[[sync]]
include = ["target/release/my-tool"]
destination = "~/bin/"
```

### Web Application

```toml
[project]
name = "my-webapp"

# Sync built frontend assets
[[sync]]
name = "static-assets"
include = ["dist/**/*"]
destination = "/var/www/myapp/"
mirror = true

# Sync config files
[[sync]]
name = "configs"
include = ["config/*.json", "config/*.yaml"]
destination = "/etc/myapp/"
```

### Multi-Platform Build

```toml
[project]
name = "cross-platform-app"

# Windows builds
[[sync]]
name = "windows"
include = ["target/x86_64-pc-windows-gnu/release/*.exe"]
destination = "releases/windows/"
clients = ["windows-*"]

# Linux builds
[[sync]]
name = "linux"
include = ["target/x86_64-unknown-linux-gnu/release/myapp"]
destination = "releases/linux/"
clients = ["linux-*"]

# macOS builds
[[sync]]
name = "macos"
include = ["target/x86_64-apple-darwin/release/myapp"]
destination = "releases/macos/"
clients = ["macos-*"]
```

## Usage

### Config-Based Syncing

Start automatic syncing with filesystem watching:

```bash
# Sync using .hrlauncher.toml from current directory or ancestors
halfremembered-launcher config-sync --server user@host --port 20222
```

The command will:
1. Load `.hrlauncher.toml` configuration
2. Connect to the server
3. Set up filesystem watches for all include patterns
4. Sync changes automatically as files are modified
5. Run continuously until interrupted (Ctrl+C)

### Manual Syncing

For one-off syncs without watching, use the `sync` command:

```bash
# Sync a single file
halfremembered-launcher sync myfile.exe --destination bin/ --server user@host

# Sync will respect .hrlauncher.toml if present
```

## Workflow Example

Typical development workflow with config-based syncing:

```bash
# Terminal 1: Start HalfRemembered server
halfremembered-launcher server --port 20222

# Terminal 2: Start client on dev machine
halfremembered-launcher client user@buildhost --port 20222

# Terminal 3: Start config-based sync on build machine
cd ~/projects/my-bevy-game
halfremembered-launcher config-sync --server user@localhost --port 20222

# Now edit code, build, and changes sync automatically!
cargo build --target x86_64-pc-windows-msvc
# .exe and .dll automatically sync to Windows client

# Edit assets
vi assets/textures/player.png
# Asset automatically syncs to all clients
```

## Validation

Config files are validated on load. Common errors:

```toml
# ERROR: No sync rules
[project]
name = "test"
# Must have at least one [[sync]] block

# ERROR: Empty include
[[sync]]
include = []  # Must have at least one pattern
destination = "."

# ERROR: Empty destination
[[sync]]
include = ["*.rs"]
destination = ""  # Cannot be empty
```

## Platform Considerations

### Path Separators

Use forward slashes (`/`) in patterns and destinations. They work on all platforms:

```toml
# Good - works everywhere
include = ["src/**/*.rs"]
destination = "bin/"

# Avoid - Windows-specific
include = ["src\\**\\*.rs"]  # Don't do this
```

### Home Directory

`~/` expands to the client's home directory in a platform-appropriate way:

```toml
destination = "~/bin/"
# Linux/macOS: /home/user/bin/
# Windows: C:\Users\user\bin\
```

### Absolute Paths

Absolute paths should generally be avoided in portable configs, but if needed:

```toml
# Will only work on Unix-like systems
destination = "/opt/myapp/"

# Will only work on Windows
destination = "C:/Program Files/myapp/"
```

## Troubleshooting

### Config Not Found

```
Error: No .hrlauncher.toml found in current directory or any parent directory
```

**Solution:** Create a `.hrlauncher.toml` file or cd to a directory containing one.

### No Files Syncing

**Check:**
1. Are your include patterns correct? Test with `ls` or `find`
2. Are files being created/modified in watched paths?
3. Are exclude patterns blocking your files?
4. Are clients connected? Check with `halfremembered-launcher list`

### Permission Errors

```
Error: Failed to write file: Permission denied
```

**Solution:** Ensure the client has write permissions to the destination directory. The client daemon runs with user permissions.

## Advanced Topics

### Performance

- Filesystem watching uses platform-native APIs (inotify on Linux, ReadDirectoryChangesW on Windows, FSEvents on macOS)
- Changes are debounced with a 100ms window to batch rapid edits
- Only changed blocks are synced via rsync algorithm (not entire files)
- Multiple sync rules are processed independently

### Security

- All syncing happens over SSH with agent authentication
- Paths are validated to prevent directory traversal attacks
- Checksums verify file integrity after transfer
- Client runs in user space with user permissions (no elevation)

## See Also

- [CLAUDE.md](CLAUDE.md) - Project architecture and design principles
- [botdocs/01-feat-notify-plan.md](botdocs/01-feat-notify-plan.md) - Filesystem watching implementation plan
- [protocol/src/lib.rs](protocol/src/lib.rs) - Protocol message definitions
