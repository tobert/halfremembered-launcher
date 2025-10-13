mod client_daemon;
mod client_registry;
mod rsync_utils;
mod ssh_client;
mod ssh_server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
                .with_agent_socket(agent_socket);

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
                LocalResponse::Success { message } => {
                    println!("{}", message);
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
