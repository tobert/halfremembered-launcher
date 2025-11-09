use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use halfremembered_launcher::{client_daemon, config, ssh_client, ssh_server};
use halfremembered_protocol::{LocalCommand, LocalResponse};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "halfremembered-launcher")]
#[command(about = "SSH-based remote build launcher with persistent connections", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the SSH server (accepts client connections)
    Server {
        /// Port to listen on
        #[arg(short, long, default_value = "20222")]
        port: u16,
    },

    /// Start the client daemon (connects to server)
    Client {
        /// Server connection string (user@host or user@host:port)
        server: String,

        /// Server port (default: 20222)
        #[arg(short, long, default_value = "20222")]
        port: u16,

        /// Heartbeat interval in seconds
        #[arg(long, default_value = "30")]
        heartbeat: u64,

        /// Reconnect delay in seconds
        #[arg(long, default_value = "5")]
        reconnect: u64,

        /// SSH agent socket path (Unix: socket path, Windows: named pipe path)
        /// Defaults to SSH_AUTH_SOCK env var on Unix, \\.\pipe\openssh-ssh-agent on Windows
        #[arg(long)]
        agent_socket: Option<String>,

        /// Disable initial sync of watched files on connection
        #[arg(long, default_value = "false")]
        no_initial_sync: bool,
    },

    /// Send ping to a connected client (server-side command)
    Ping {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// Hostname of the client to ping
        hostname: String,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// List connected clients (server-side command)
    List {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Execute command on a connected client (server-side command)
    Exec {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// Hostname of the client to execute on
        hostname: String,

        /// Binary to execute
        binary: String,

        /// Arguments for the binary
        args: Vec<String>,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Sync file to all connected clients (server-side command)
    Sync {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// Local file path to sync
        file: PathBuf,

        /// Remote destination path on clients
        #[arg(short, long)]
        destination: Option<String>,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Push binary to remote host via scp
    Push {
        /// Server connection string (user@host)
        server: String,

        /// Local binary path to upload
        #[arg(
            short,
            long,
            default_value = "./target/release/halfremembered-launcher"
        )]
        binary: PathBuf,

        /// Remote destination path
        #[arg(short, long, default_value = "~/halfremembered-launcher")]
        destination: String,

        /// Start the server after uploading
        #[arg(long)]
        start: bool,

        /// Server port for control connection (if --start is used)
        #[arg(short, long, default_value = "20222")]
        port: u16,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Get server status (server-side command)
    Status {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Shutdown the server (server-side command)
    Shutdown {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Watch a file or directory for changes and auto-sync to clients (server-side command)
    Watch {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// File or directory to watch
        path: PathBuf,

        /// Watch recursively (only applies to directories)
        #[arg(short, long, default_value = "true")]
        recursive: bool,

        /// Include patterns (e.g., "*.rs", "*.toml")
        #[arg(long)]
        include: Vec<String>,

        /// Exclude patterns (e.g., "*.tmp", "target/*")
        #[arg(long)]
        exclude: Vec<String>,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Stop watching a file or directory (server-side command)
    Unwatch {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// File or directory to stop watching
        path: PathBuf,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// List active filesystem watches (server-side command)
    ListWatches {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },

    /// Sync files using .hrlauncher.toml config with automatic filesystem watching
    ConfigSync {
        /// Server connection string (user@host or just host, defaults to $USER@localhost)
        #[arg(short, long)]
        server: Option<String>,

        /// Server port
        #[arg(short = 'P', long, default_value = "20222")]
        port: u16,

        /// Path to config file (default: search for .hrlauncher.toml in current dir and parents)
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// SSH agent socket path
        #[arg(long)]
        agent_socket: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server { port } => {
            log::info!("Starting HalfRemembered server on port {}", port);
            ssh_server::SshServer::run(port).await?;
        }

        Commands::Client {
            server,
            port,
            heartbeat,
            reconnect,
            agent_socket,
            no_initial_sync,
        } => {
            log::info!("Starting HalfRemembered client, connecting to {}", server);

            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let hostname = hostname::get()
                .context("Failed to get hostname")?
                .to_string_lossy()
                .to_string();

            let mut daemon = client_daemon::ClientDaemon::new(host, final_port, user, hostname)
                .with_heartbeat_interval(std::time::Duration::from_secs(heartbeat))
                .with_reconnect_delay(std::time::Duration::from_secs(reconnect))
                .with_agent_socket(agent_socket)
                .with_initial_sync(!no_initial_sync);

            daemon.run().await?;
        }

        Commands::Ping {
            server,
            port,
            hostname,
            agent_socket,
        } => {
            log::info!("Pinging client: {}", hostname);

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::Ping {
                target: hostname.clone(),
            };

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::List {
            server,
            port,
            agent_socket,
        } => {
            log::debug!("Listing connected clients");

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::ListClients;

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::ClientList { clients } => {
                    if clients.is_empty() {
                        println!("No clients connected");
                    } else {
                        println!("Connected clients ({}):", clients.len());
                        for client in clients {
                            println!(
                                "  {} - {} ({})",
                                client.hostname, client.platform, client.session_id
                            );
                        }
                    }
                }
                LocalResponse::Error { message } => {
                    eprintln!("Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Exec {
            server,
            port,
            hostname,
            binary,
            args,
            agent_socket,
        } => {
            log::info!("Executing {} on {}", binary, hostname);

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::Execute {
                target: hostname.clone(),
                binary,
                args,
            };

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Sync {
            server,
            port,
            file,
            destination,
            agent_socket,
        } => {
            log::info!("Syncing {} to all clients", file.display());

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let dest = destination.unwrap_or_else(|| file.to_string_lossy().to_string());

            let command = LocalCommand::SyncFile {
                file: file.to_string_lossy().to_string(),
                destination: dest,
            };

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Push {
            server,
            binary,
            destination,
            start,
            port,
            agent_socket,
        } => {
            log::info!("Pushing {} to {}", binary.display(), server);

            let (user, host, _conn_port) = parse_connection_string(&server)?;

            // Upload binary via SFTP (uses host sshd on port 22)
            ssh_client::SshClientConnection::upload_file_via_sftp(
                &host,
                22, // Use standard SSH port for upload
                &user,
                &binary,
                &destination,
                agent_socket.as_deref(),
            )
            .await?;

            println!(
                "✓ Uploaded {} to {}@{}:{}",
                binary.display(),
                user,
                host,
                destination
            );

            if start {
                log::info!("Starting server on remote host");

                // Make the binary executable using russh
                let chmod_cmd = format!("chmod +x {}", destination);
                let (chmod_success, _, chmod_stderr) =
                    ssh_client::SshClientConnection::execute_remote_command(
                        &host,
                        22, // Use standard SSH port
                        &user,
                        &chmod_cmd,
                        agent_socket.as_deref(),
                    )
                    .await
                    .context("Failed to set executable permission")?;

                if !chmod_success && !chmod_stderr.is_empty() {
                    log::warn!("chmod failed: {}", chmod_stderr);
                }

                // Start the server in the background using russh
                let start_cmd = format!(
                    "nohup {} server --port {} >/dev/null 2>&1 </dev/null &",
                    destination, port
                );
                let (start_success, start_stdout, start_stderr) =
                    ssh_client::SshClientConnection::execute_remote_command(
                        &host,
                        22, // Use standard SSH port
                        &user,
                        &start_cmd,
                        agent_socket.as_deref(),
                    )
                    .await
                    .context("Failed to start server")?;

                if start_success || start_stdout.is_empty() && start_stderr.is_empty() {
                    println!("✓ Started server on {}:{}", host, port);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    println!("  Server should be listening on port {}", port);
                } else if !start_stderr.is_empty() {
                    eprintln!("✗ Failed to start server: {}", start_stderr);
                } else {
                    println!("✓ Server start command issued on {}:{}", host, port);
                }
            }
        }

        Commands::Status {
            server,
            port,
            agent_socket,
        } => {
            log::debug!("Getting server status");

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::Status;

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Status {
                    hostname,
                    version,
                    uptime,
                    clients,
                } => {
                    println!("Server: {}", hostname);
                    println!("Version: {}", version);
                    println!("Uptime: {}", format_duration(uptime));
                    println!("Connected clients: {}", clients.len());

                    if !clients.is_empty() {
                        println!();
                        println!("Clients:");
                        for client in clients {
                            let client_uptime = format_duration(client.connected_at);
                            println!(
                                "  {} ({}) - uptime: {}, last heartbeat: {}s ago",
                                client.hostname, client.platform, client_uptime, client.last_heartbeat
                            );
                        }
                    }
                }
                LocalResponse::Error { message } => {
                    eprintln!("Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Shutdown {
            server,
            port,
            agent_socket,
        } => {
            log::info!("Shutting down server");

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::Shutdown;

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Watch {
            server,
            port,
            path,
            recursive,
            include,
            exclude,
            agent_socket,
        } => {
            log::info!("Adding watch for path: {}", path.display());

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::WatchDirectory {
                path: path.to_string_lossy().to_string(),
                recursive,
                include_patterns: include,
                exclude_patterns: exclude,
            };

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::Unwatch {
            server,
            port,
            path,
            agent_socket,
        } => {
            log::info!("Removing watch for path: {}", path.display());

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::UnwatchDirectory {
                path: path.to_string_lossy().to_string(),
            };

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::Success { message } => {
                    println!("✓ {}", message);
                }
                LocalResponse::Error { message } => {
                    eprintln!("✗ Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("✗ Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::ListWatches {
            server,
            port,
            agent_socket,
        } => {
            log::debug!("Listing active watches");

            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);
            let command = LocalCommand::ListWatches;

            let response = ssh_client::SshClientConnection::send_control_command(
                &host,
                final_port,
                &user,
                command,
                agent_socket.as_deref(),
            )
            .await?;

            match response {
                LocalResponse::WatchList { watches } => {
                    if watches.is_empty() {
                        println!("No active watches");
                    } else {
                        println!("Active watches ({}):", watches.len());
                        for watch in watches {
                            println!("  {} (recursive: {})", watch.path, watch.recursive);
                            if !watch.include_patterns.is_empty() {
                                println!("    Include: {:?}", watch.include_patterns);
                            }
                            if !watch.exclude_patterns.is_empty() {
                                println!("    Exclude: {:?}", watch.exclude_patterns);
                            }
                        }
                    }
                }
                LocalResponse::Error { message } => {
                    eprintln!("Error: {}", message);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Unexpected response: {:?}", response);
                    std::process::exit(1);
                }
            }
        }

        Commands::ConfigSync {
            server,
            port,
            config,
            agent_socket,
        } => {
            // Load config from specified path or search for it
            let (config_path, config) = if let Some(path) = config {
                let cfg = config::Config::from_file(&path)?;
                (path, cfg)
            } else {
                config::Config::find_and_load()?
            };

            log::info!("Loaded config from: {}", config_path.display());
            log::info!("Project: {}", config.project.name);
            log::info!("Sync rules: {}", config.sync_rules.len());

            println!("✓ Loaded config: {}", config.project.name);
            println!("  Config file: {}", config_path.display());
            println!("  Sync rules: {}", config.sync_rules.len());
            println!();

            // Get config file's parent directory (the project root)
            let project_root = config_path
                .parent()
                .context("Config file has no parent directory")?
                .to_path_buf();

            log::info!("Project root: {}", project_root.display());

            // Connect to server
            let server = server.unwrap_or_else(|| format!("{}@localhost", get_default_user().unwrap()));
            let (user, host, conn_port) = parse_connection_string(&server)?;
            let final_port = conn_port.unwrap_or(port);

            println!("Setting up watches on server {}@{}:{}...", user, host, final_port);
            println!();

            // For each sync rule, set up a watch on the server
            for (idx, rule) in config.sync_rules.iter().enumerate() {
                let default_name = format!("rule-{}", idx + 1);
                let rule_name = rule.name.as_deref().unwrap_or(&default_name);

                log::info!(
                    "Setting up watch for [{}]: {} -> {}",
                    rule_name,
                    project_root.display(),
                    rule.destination
                );

                // Send WatchDirectory command to server
                // Watch the project root with include/exclude patterns
                let command = LocalCommand::WatchDirectory {
                    path: project_root.to_string_lossy().to_string(),
                    recursive: true,
                    include_patterns: rule.include.clone(),
                    exclude_patterns: rule.exclude.clone(),
                };

                let response = ssh_client::SshClientConnection::send_control_command(
                    &host,
                    final_port,
                    &user,
                    command,
                    agent_socket.as_deref(),
                )
                .await?;

                match response {
                    LocalResponse::Success { message } => {
                        println!("  ✓ [{}] {}", rule_name, message);
                        println!("      Include: {:?}", rule.include);
                        if !rule.exclude.is_empty() {
                            println!("      Exclude: {:?}", rule.exclude);
                        }
                        println!("      Destination: {}", rule.destination);
                    }
                    LocalResponse::Error { message } => {
                        eprintln!("  ✗ [{}] Error: {}", rule_name, message);
                        std::process::exit(1);
                    }
                    _ => {
                        eprintln!("  ✗ [{}] Unexpected response: {:?}", rule_name, response);
                        std::process::exit(1);
                    }
                }
            }

            println!();
            println!("✓ All watches configured successfully!");
            println!();
            println!("The server is now watching for file changes and will automatically");
            println!("sync them to connected clients. File changes will be logged on the server.");
            println!();
            println!("To view active watches, run:");
            println!("  halfremembered-launcher list-watches --server {}@{}", user, host);
            println!();
            println!("To stop a watch, run:");
            println!("  halfremembered-launcher unwatch <directory> --server {}@{}", user, host);
        }
    }

    Ok(())
}

fn get_default_user() -> Result<String> {
    // Try USER first (Unix/Linux/WSL)
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        return Ok(user);
    }

    // Fall back to USERNAME (Windows)
    if let Ok(user) = std::env::var("USERNAME")
        && !user.is_empty()
    {
        return Ok(user);
    }

    anyhow::bail!(
        "Could not determine username. Neither USER nor USERNAME environment variables are set."
    )
}

fn parse_connection_string(connection: &str) -> Result<(String, String, Option<u16>)> {
    let parts: Vec<&str> = connection.split('@').collect();

    let (user, host, port) = match parts.len() {
        1 => {
            // No '@' found, use default user
            let user = get_default_user()?;
            let (host, port) = parse_host_port(parts[0])?;
            (user, host, port)
        }
        2 => {
            // user@host format
            let user = parts[0].to_string();
            let (host, port) = parse_host_port(parts[1])?;
            (user, host, port)
        }
        _ => {
            anyhow::bail!(
                "Invalid connection string. Expected format: host, user@host, or user@host:port"
            );
        }
    };

    Ok((user, host, port))
}

fn parse_host_port(host_str: &str) -> Result<(String, Option<u16>)> {
    let parts: Vec<&str> = host_str.split(':').collect();
    match parts.len() {
        1 => Ok((parts[0].to_string(), None)),
        2 => {
            let host = parts[0].to_string();
            let port = parts[1]
                .parse::<u16>()
                .context("Invalid port number in connection string")?;
            Ok((host, Some(port)))
        }
        _ => {
            anyhow::bail!("Invalid host:port format");
        }
    }
}

fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {}h {}m", days, hours, minutes)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}
