# HalfRemembered Launcher 🚀

> Push files and run programs on remote machines through persistent SSH connections

HalfRemembered Launcher is a tool that lets you push files and execute commands on remote machines that connect to your build server. Think of it as "reverse SSH" - clients behind NAT or firewalls connect to your server, then you can push builds and launch programs on them. Perfect for deploying game builds, syncing binaries to test machines, or running commands across your fleet.

**Key features:**
- 🔒 **Secure**: Everything over SSH, no custom protocols
- 📡 **NAT-friendly**: Clients connect outbound only (no port forwarding needed)
- ⚡ **Efficient**: Uses rsync algorithm to transfer only changed data
- 🎮 **User context**: Runs in your environment with access to graphics, audio, etc.
- 🪟 **Cross-platform**: Linux, WSL, and Windows
- ↩️ **Reversible**: Executables keep deploy history, so a bad build rolls back on the machine itself

## Quick Start

**Prerequisites:** SSH agent running with a key loaded (`ssh-add -l` should show your key)

```bash
# 1. Build the project
cargo build --release

# 2. Start the server (in one terminal)
./target/release/halfremembered-launcher server

# 3. Connect a client (in another terminal)
./target/release/halfremembered-launcher client localhost

# 4. Push a file from a third terminal
./target/release/halfremembered-launcher sync Cargo.toml \
    --destination /tmp/Cargo.toml \
    --server localhost
```

That's it! Check `/tmp/Cargo.toml` on your client - the file was pushed through the persistent SSH connection.

## Installation

```bash
# Install to ~/.cargo/bin
cargo install --path launcher

# Now you can just run:
halfremembered-launcher server
```

For Windows builds and cross-compilation, see [docs/CROSS_COMPILE.md](docs/CROSS_COMPILE.md).

## Basic Usage

### Running the Server

Start the server on a machine that clients can reach:

```bash
halfremembered-launcher server              # Default port 20222
halfremembered-launcher server --port 1337  # Custom port
```

### Connecting Clients

Clients establish outbound SSH connections to the server:

```bash
halfremembered-launcher client localhost                    # Same machine
halfremembered-launcher client user@example.com            # Remote server
halfremembered-launcher client user@server.local:1337     # Custom port
```

### Managing Clients

Use these commands to interact with connected clients (run from any machine that can SSH to the server):

```bash
# List connected clients
halfremembered-launcher list --server user@localhost

# Push a file to all clients
halfremembered-launcher sync ./my-game.exe \
    --destination ~/games/my-game.exe \
    --server user@localhost

# Run a command on a specific client
halfremembered-launcher exec laptop01 ~/games/my-game.exe \
    --server user@localhost
```

### Undoing a Bad Deploy

Executables are installed with version history, so a bad build can be undone
on the machine itself:

```bash
# Run these ON the machine holding the file
halfremembered-launcher versions ~/games/my-game   # what was deployed, and when
halfremembered-launcher rollback ~/games/my-game   # put the previous one back
```

These are deliberately **local** commands. They need no server, no network and
no build machine, because the case they exist for is the one where the thing
you broke is how you reach the box. The previous version is stored beside the
file and verified against its checksum before it is put back — a corrupted
stored version is refused rather than installed over a working binary.

Every install is atomic: the destination is never a partial file, never
briefly missing, and never briefly non-executable. Executables are also
checked against the target's glibc before activation and refused if that
machine could not run them, which turns a program that dies on start into a
deploy that fails loudly.

## Platform Setup

### Linux

```bash
# Ensure SSH agent is running
eval $(ssh-agent)
ssh-add ~/.ssh/id_ed25519
ssh-add -l  # Verify
```

### Windows

```powershell
# Start OpenSSH agent service
Get-Service ssh-agent | Set-Service -StartupType Automatic
Start-Service ssh-agent

# Add your key
ssh-add C:\Users\YourName\.ssh\id_ed25519
ssh-add -l  # Verify
```

## Use Cases

- **Game Development**: Build on your workstation, auto-deploy to test machines
- **Cross-Platform Testing**: Push builds to Windows, Linux, and WSL simultaneously
- **Remote Execution**: Launch programs on machines behind NAT/firewalls
- **Lab Management**: Sync tools and run commands across multiple machines

## Documentation

- [Building and Installation](docs/BUILDING.md) - Detailed build instructions
- [Cross-Compilation for Windows](docs/CROSS_COMPILE.md) - Using xwin for Windows builds
- [Architecture](docs/ARCHITECTURE.md) - How it works under the hood
- [Configuration](CONFIG.md) - Auto-sync configuration with `.hrlauncher.toml`
- [Agent Contributors](AGENTS.md) - Guide for AI coding assistants

## How It Works

Clients establish persistent SSH connections to the server. The server multiplexes multiple operations (file transfers, commands, heartbeats) over each connection. When you push a file, the server uses rsync delta algorithm to transfer only the changed parts, then the client writes it atomically to disk.

```
┌─────────────┐         SSH          ┌─────────────┐
│   Client    │◄─────────────────────┤   Server    │
│  (behind    │  Persistent tunnel   │  (port      │
│   NAT)      │  + multiplexed ops   │   20222)    │
└─────────────┘                      └─────────────┘
```

For technical details, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Troubleshooting

**Connection issues?**
```bash
# Check SSH agent
ssh-add -l

# Enable debug logging
RUST_LOG=debug halfremembered-launcher client user@server

# Test SSH connectivity
ssh -p 20222 user@server
```

**Windows path issues?**
- Use forward slashes: `/c/Users/name/file.txt` or `C:/Users/name/file.txt`
- Or Windows style: `C:\Users\name\file.txt` (backslashes work too)

## About

Created by [Amy Tobey](https://github.com/tobert) with [Claude Code](https://claude.ai/code) and [Gemini](https://gemini.google.com). An experiment in AI-assisted open source development and shell script harm reduction.

## License

MIT - See [LICENSE](LICENSE)
