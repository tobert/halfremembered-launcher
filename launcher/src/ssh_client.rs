use anyhow::{Context, Result};
use halfremembered_protocol::{
    ClientMessage, Frame, LocalCommand, LocalResponse, MessageBuffer, ServerMessage,
    FRAME_HEADER_SIZE,
};
use russh::client::{self, Handle};
use russh::keys;
use russh::*;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[cfg(unix)]
type PlatformAgentClient = keys::agent::client::AgentClient<tokio::net::UnixStream>;

#[cfg(windows)]
type PlatformAgentClient =
    keys::agent::client::AgentClient<tokio::net::windows::named_pipe::NamedPipeClient>;

pub struct SshClientConnection {
    session: Handle<ClientHandler>,
    channel: Arc<Mutex<Option<Channel<client::Msg>>>>,
    message_buffer: Arc<Mutex<MessageBuffer>>,
}

pub struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept all server keys for now (ephemeral keys)
        Ok(true)
    }
}

/// Connect to ssh-agent with platform-specific handling
#[cfg(unix)]
async fn connect_agent(agent_socket: Option<&str>) -> Result<PlatformAgentClient> {
    match agent_socket {
        Some(path) => {
            log::debug!("Connecting to ssh-agent at: {}", path);
            PlatformAgentClient::connect_uds(path)
                .await
                .context(format!("Failed to connect to ssh-agent at {}", path))
        }
        None => {
            log::debug!("Connecting to ssh-agent via SSH_AUTH_SOCK");
            PlatformAgentClient::connect_env()
                .await
                .context("Failed to connect to ssh-agent. Make sure ssh-agent is running and SSH_AUTH_SOCK is set")
        }
    }
}

#[cfg(windows)]
async fn connect_agent(agent_socket: Option<&str>) -> Result<PlatformAgentClient> {
    let pipe_path = agent_socket.unwrap_or(r"\\.\pipe\openssh-ssh-agent");
    log::debug!("Connecting to ssh-agent at: {}", pipe_path);
    PlatformAgentClient::connect_named_pipe(pipe_path)
        .await
        .context(format!("Failed to connect to ssh-agent named pipe at {}. Make sure OpenSSH authentication agent service is running", pipe_path))
}

impl SshClientConnection {
    pub async fn connect(
        host: &str,
        port: u16,
        user: &str,
        agent_socket: Option<&str>,
    ) -> Result<Self> {
        log::info!("Connecting to {}:{} as {}", host, port, user);

        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            ..Default::default()
        };

        let config = Arc::new(config);
        let message_buffer = Arc::new(Mutex::new(MessageBuffer::new()));

        let handler = ClientHandler;

        let mut session = client::connect(config, (host, port), handler).await?;

        // Try to authenticate using ssh-agent
        log::info!("Attempting authentication with ssh-agent");

        let mut agent = connect_agent(agent_socket).await?;

        let identities = agent
            .request_identities()
            .await
            .context("Failed to list ssh-agent identities")?;

        log::debug!("Found {} identities in ssh-agent", identities.len());

        if identities.is_empty() {
            anyhow::bail!("No identities found in ssh-agent. Add a key with: ssh-add");
        }

        let mut authenticated = false;
        for public_key in identities {
            log::debug!(
                "Trying key fingerprint: {}",
                public_key.fingerprint(keys::HashAlg::Sha256)
            );

            match session
                .authenticate_publickey_with(user, public_key, None, &mut agent)
                .await
            {
                Ok(auth_result) if auth_result.success() => {
                    log::info!("Successfully authenticated with ssh-agent");
                    authenticated = true;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    log::debug!("Auth attempt failed: {:?}", e);
                    continue;
                }
            }
        }

        if !authenticated {
            anyhow::bail!("All ssh-agent identities rejected by server");
        }

        log::info!("SSH connection established");

        // Open a session channel for our protocol
        let channel = session
            .channel_open_session()
            .await
            .context("Failed to open session channel")?;

        Ok(Self {
            session,
            channel: Arc::new(Mutex::new(Some(channel))),
            message_buffer,
        })
    }

    pub async fn send_message(&self, msg: &ClientMessage) -> Result<()> {
        let mut full_message = Vec::new();
        msg.write_framed(&mut full_message)?;

        let mut channel_guard = self.channel.lock().await;
        let channel = channel_guard.as_mut().context("Channel not available")?;

        channel
            .data(&full_message[..])
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send data: {:?}", e))?;

        log::debug!("Sent message: {}", msg.message_type());
        Ok(())
    }

    pub async fn send_register(&self, hostname: &str) -> Result<()> {
        let platform = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "unknown"
        };

        let msg = ClientMessage::Register {
            hostname: hostname.to_string(),
            platform: platform.to_string(),
        };

        self.send_message(&msg).await
    }

    pub async fn send_heartbeat(&self, sequence: u32) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let msg = ClientMessage::Heartbeat {
            timestamp,
            sequence,
        };

        self.send_message(&msg).await
    }

    pub async fn try_receive_message(&self) -> Result<Option<ServerMessage>> {
        let mut channel_guard = self.channel.lock().await;
        let channel = channel_guard.as_mut().context("Channel not available")?;

        // Try to receive with a short timeout
        let timeout = tokio::time::Duration::from_millis(10);
        match tokio::time::timeout(timeout, channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                let mut buffer = self.message_buffer.lock().await;
                buffer.append(&data);

                match buffer.try_parse_server_message() {
                    Ok(Some(msg)) => {
                        log::debug!("Received message: {}", msg.message_type());
                        Ok(Some(msg))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                }
            }
            Ok(Some(ChannelMsg::Eof)) => {
                anyhow::bail!("Channel EOF received")
            }
            Ok(Some(ChannelMsg::Close)) => {
                anyhow::bail!("Channel closed by server")
            }
            Ok(Some(msg)) => {
                log::debug!("Received other channel message: {:?}", msg);
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(_) => Ok(None), // Timeout - no message available
        }
    }

    pub async fn upload_file_via_sftp(
        host: &str,
        port: u16,
        user: &str,
        local_path: &Path,
        remote_path: &str,
        agent_socket: Option<&str>,
    ) -> Result<()> {
        log::info!(
            "Uploading {} to {}@{}:{}",
            local_path.display(),
            user,
            host,
            remote_path
        );

        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };

        let config = Arc::new(config);

        let handler = ClientHandler;

        let mut session = client::connect(config, (host, port), handler)
            .await
            .context(format!("Failed to connect to {}:{}", host, port))?;

        // Authenticate using ssh-agent
        let mut agent = connect_agent(agent_socket).await?;

        let identities = agent
            .request_identities()
            .await
            .context("Failed to list ssh-agent identities")?;

        if identities.is_empty() {
            anyhow::bail!("No identities found in ssh-agent. Add a key with: ssh-add");
        }

        let mut authenticated = false;
        for public_key in identities {
            match session
                .authenticate_publickey_with(user, public_key, None, &mut agent)
                .await
            {
                Ok(auth_result) if auth_result.success() => {
                    authenticated = true;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    log::debug!("Auth attempt failed: {:?}", e);
                    continue;
                }
            }
        }

        if !authenticated {
            anyhow::bail!("All ssh-agent identities rejected by server");
        }

        // Open SFTP channel
        let sftp_channel = session
            .channel_open_session()
            .await
            .context("Failed to open SFTP channel")?;

        sftp_channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| anyhow::anyhow!("Failed to request SFTP subsystem: {:?}", e))?;

        let sftp = SftpSession::new(sftp_channel.into_stream())
            .await
            .context("Failed to create SFTP session")?;

        // Read local file
        let contents = tokio::fs::read(local_path).await.context(format!(
            "Failed to read local file: {}",
            local_path.display()
        ))?;

        // Create remote file
        let mut file = sftp
            .create(remote_path)
            .await
            .context(format!("Failed to create remote file: {}", remote_path))?;

        // Write contents
        file.write_all(&contents)
            .await
            .context("Failed to write to remote file")?;

        log::info!("Uploaded {} bytes to {}", contents.len(), remote_path);

        // Close SFTP session
        sftp.close().await.context("Failed to close SFTP session")?;

        // Clean disconnect
        let _ = session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await;

        Ok(())
    }

    pub async fn execute_remote_command(
        host: &str,
        port: u16,
        user: &str,
        command: &str,
        agent_socket: Option<&str>,
    ) -> Result<(bool, String, String)> {
        log::debug!("Executing remote command on {}:{}: {}", host, port, command);

        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };

        let config = Arc::new(config);

        let handler = ClientHandler;

        let mut session = client::connect(config, (host, port), handler)
            .await
            .context(format!("Failed to connect to {}:{}", host, port))?;

        // Authenticate using ssh-agent
        let mut agent = connect_agent(agent_socket).await?;

        let identities = agent
            .request_identities()
            .await
            .context("Failed to list ssh-agent identities")?;

        if identities.is_empty() {
            anyhow::bail!("No identities found in ssh-agent. Add a key with: ssh-add");
        }

        let mut authenticated = false;
        for public_key in identities {
            match session
                .authenticate_publickey_with(user, public_key, None, &mut agent)
                .await
            {
                Ok(auth_result) if auth_result.success() => {
                    authenticated = true;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    log::debug!("Auth attempt failed: {:?}", e);
                    continue;
                }
            }
        }

        if !authenticated {
            anyhow::bail!("All ssh-agent identities rejected by server");
        }

        // Open an exec channel
        let mut channel = session
            .channel_open_session()
            .await
            .context("Failed to open session channel")?;

        // Execute the command
        channel
            .exec(true, command)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute command: {:?}", e))?;

        // Collect output
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    stdout.extend_from_slice(&data);
                }
                Some(ChannelMsg::ExtendedData { data, ext }) => {
                    if ext == 1 {
                        stderr.extend_from_slice(&data);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    let success = exit_status == 0;
                    let stdout_str = String::from_utf8_lossy(&stdout).to_string();
                    let stderr_str = String::from_utf8_lossy(&stderr).to_string();

                    // Clean disconnect
                    let _ = channel.eof().await;
                    let _ = session
                        .disconnect(Disconnect::ByApplication, "", "English")
                        .await;

                    return Ok((success, stdout_str, stderr_str));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                    break;
                }
                _ => {}
            }
        }

        let stdout_str = String::from_utf8_lossy(&stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&stderr).to_string();

        // Clean disconnect
        let _ = session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await;

        Ok((false, stdout_str, stderr_str))
    }

    pub async fn send_control_command(
        host: &str,
        port: u16,
        user: &str,
        command: LocalCommand,
        agent_socket: Option<&str>,
    ) -> Result<LocalResponse> {
        log::debug!("Sending control command to {}:{}", host, port);

        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };

        let config = Arc::new(config);

        let handler = ClientHandler;

        let mut session = client::connect(config, (host, port), handler)
            .await
            .context(format!("Failed to connect to {}:{}", host, port))?;

        // Authenticate using ssh-agent
        let mut agent = connect_agent(agent_socket).await?;

        let identities = agent
            .request_identities()
            .await
            .context("Failed to list ssh-agent identities")?;

        if identities.is_empty() {
            anyhow::bail!("No identities found in ssh-agent. Add a key with: ssh-add");
        }

        let mut authenticated = false;
        for public_key in identities {
            match session
                .authenticate_publickey_with(user, public_key, None, &mut agent)
                .await
            {
                Ok(auth_result) if auth_result.success() => {
                    authenticated = true;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    log::debug!("Auth attempt failed: {:?}", e);
                    continue;
                }
            }
        }

        if !authenticated {
            anyhow::bail!("All ssh-agent identities rejected by server");
        }

        // Open a session channel
        let mut channel = session
            .channel_open_session()
            .await
            .context("Failed to open session channel")?;

        // Send command
        let mut full_message = Vec::new();
        command
            .write_framed(&mut full_message)
            .context("Failed to serialize command")?;

        channel
            .data(&full_message[..])
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send command: {:?}", e))?;

        log::debug!("Command sent, waiting for response");

        // Wait for response with timeout
        let timeout = tokio::time::Duration::from_secs(30);
        let response = tokio::time::timeout(timeout, async {
            let mut buffer = MessageBuffer::new();
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => {
                        buffer.append(&data);
                        if let Some(resp) = buffer.try_parse_local_response()? {
                            return Ok(resp);
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                        anyhow::bail!("Channel closed before receiving response")
                    }
                    Some(msg) => {
                        log::debug!("Received other channel message: {:?}", msg);
                    }
                    None => {
                        anyhow::bail!("Channel ended without response")
                    }
                }
            }
        })
        .await
        .context("Timeout waiting for response")??;

        // Clean disconnect
        let _ = channel.eof().await;
        let _ = session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await;

        Ok(response)
    }

    /// Open a dedicated rsync channel
    /// Returns a new channel for rsync data transfer
    pub async fn open_rsync_channel(&self) -> Result<Channel<client::Msg>> {
        self.session
            .channel_open_session()
            .await
            .context("Failed to open rsync channel")
    }

    /// Read a frame from a channel
    /// Blocks until a complete frame is received
    pub async fn read_frame_from_channel(
        channel: &mut Channel<client::Msg>,
    ) -> Result<Frame> {
        let mut buffer = Vec::new();

        // Read until we have at least the header
        while buffer.len() < FRAME_HEADER_SIZE {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    buffer.extend_from_slice(&data);
                }
                Some(ChannelMsg::Eof) => {
                    anyhow::bail!("Channel EOF while reading frame")
                }
                Some(ChannelMsg::Close) => {
                    anyhow::bail!("Channel closed while reading frame")
                }
                None => {
                    anyhow::bail!("Channel ended while reading frame")
                }
                _ => continue,
            }
        }

        // Parse header to get total frame size
        let length =
            u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
        let message_type = u16::from_be_bytes([buffer[4], buffer[5]]);

        // Validate length
        if length < 2 {
            anyhow::bail!("Invalid frame length: {} (must be >= 2)", length);
        }

        let total_size = FRAME_HEADER_SIZE + length - 2; // Header + (length - message_type_size)

        // Read until we have the complete frame
        while buffer.len() < total_size {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    buffer.extend_from_slice(&data);
                }
                Some(ChannelMsg::Eof) => {
                    anyhow::bail!("Channel EOF while reading frame payload")
                }
                Some(ChannelMsg::Close) => {
                    anyhow::bail!("Channel closed while reading frame payload")
                }
                None => {
                    anyhow::bail!("Channel ended while reading frame payload")
                }
                _ => continue,
            }
        }

        // Extract the payload (everything after the header)
        let payload = buffer[FRAME_HEADER_SIZE..total_size].to_vec();

        Ok(Frame::new(message_type, payload))
    }

    /// Write a frame to a channel
    pub async fn write_frame_to_channel(
        channel: &mut Channel<client::Msg>,
        frame: &Frame,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        frame
            .write(&mut buffer)
            .context("Failed to serialize frame")?;

        channel
            .data(&buffer[..])
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send frame data: {:?}", e))?;

        Ok(())
    }
}
