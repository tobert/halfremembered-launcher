use anyhow::{Context, Result};
use halfremembered_protocol::{
    ClientMessage, Frame, FrameBuffer, LocalCommand, LocalResponse, MessageBuffer, ServerMessage,
    MSG_RSYNC_DELTA, MSG_RSYNC_SIGNATURE,
};
use rand_core::OsRng;
use russh::keys::*;
use russh::server::{Auth, Msg, Server as _, Session};
use russh::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::client_registry::{ClientRegistry, ConnectedClient};
use crate::file_watcher::FileWatcher;
use crate::rsync_utils;

/// Shared storage for rsync file data: maps request_id to (file_path, file_contents)
type RsyncFileStorage = Arc<Mutex<HashMap<String, (PathBuf, Vec<u8>)>>>;
type FileWatcherRef = Arc<Mutex<Option<FileWatcher>>>;

#[derive(Clone)]
pub struct SshServer {
    client_registry: Arc<Mutex<ClientRegistry>>,
    authorized_keys: Arc<Vec<ssh_key::PublicKey>>,
    rsync_file_storage: RsyncFileStorage,
    file_watcher: FileWatcherRef,
    start_time: Arc<Instant>,
}

impl SshServer {
    pub async fn new() -> Result<Self> {
        let authorized_keys =
            Self::load_authorized_keys().context("Failed to load authorized keys")?;

        log::info!("Loaded {} authorized keys", authorized_keys.len());

        Ok(Self {
            client_registry: Arc::new(Mutex::new(ClientRegistry::new())),
            authorized_keys: Arc::new(authorized_keys),
            rsync_file_storage: Arc::new(Mutex::new(HashMap::new())),
            file_watcher: Arc::new(Mutex::new(None)),
            start_time: Arc::new(Instant::now()),
        })
    }

    fn load_authorized_keys() -> Result<Vec<ssh_key::PublicKey>> {
        let home = std::env::var("HOME").context("HOME not set")?;
        let authorized_keys_path = PathBuf::from(home).join(".ssh/authorized_keys");

        if !authorized_keys_path.exists() {
            log::warn!("~/.ssh/authorized_keys not found, no keys loaded");
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&authorized_keys_path)
            .context("Failed to read ~/.ssh/authorized_keys")?;

        let mut keys = Vec::new();
        let mut line_number = 0;

        for line in content.lines() {
            line_number += 1;
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            match ssh_key::PublicKey::from_openssh(line) {
                Ok(key) => {
                    log::debug!(
                        "Loaded key from authorized_keys line {}: {}",
                        line_number,
                        key.fingerprint(ssh_key::HashAlg::Sha256)
                    );
                    keys.push(key);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to parse key at authorized_keys line {}: {}",
                        line_number,
                        e
                    );
                }
            }
        }

        Ok(keys)
    }

    pub async fn run(port: u16) -> Result<()> {
        let mut server = Self::new().await?;

        let host_key = russh::keys::PrivateKey::random(&mut OsRng, russh::keys::Algorithm::Ed25519)
            .context("Failed to generate ephemeral host key")?;

        log::info!("Generated ephemeral Ed25519 host key");

        let config = russh::server::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            auth_rejection_time: std::time::Duration::from_secs(3),
            auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
            keys: vec![host_key],
            ..Default::default()
        };

        log::info!("Starting SSH server on 0.0.0.0:{}", port);

        server
            .run_on_address(Arc::new(config), ("0.0.0.0", port))
            .await?;

        Ok(())
    }

    async fn handle_local_command(
        command: LocalCommand,
        registry: Arc<Mutex<ClientRegistry>>,
        rsync_storage: RsyncFileStorage,
        file_watcher: FileWatcherRef,
        start_time: Arc<Instant>,
    ) -> LocalResponse {
        match command {
            LocalCommand::Ping { target } => {
                log::info!("Ping request for client: {}", target);

                let request_id = format!("ping-{}", uuid::Uuid::new_v4());
                let ping_msg = ServerMessage::Ping {
                    request_id: request_id.clone(),
                };

                let result = registry
                    .lock()
                    .await
                    .send_to_client(&target, &ping_msg)
                    .await;

                match result {
                    Ok(_) => LocalResponse::Success {
                        message: format!("Ping sent to {}", target),
                    },
                    Err(e) => LocalResponse::Error {
                        message: format!("Failed to ping {}: {:#}", target, e),
                    },
                }
            }

            LocalCommand::ListClients => {
                log::info!("List clients request");

                let client_infos: Vec<halfremembered_protocol::ClientInfo> = {
                    let reg = registry.lock().await;
                    let clients = reg.list_clients();
                    clients
                        .iter()
                        .map(|c| halfremembered_protocol::ClientInfo {
                            hostname: c.hostname.clone(),
                            platform: c.platform.clone(),
                            session_id: c.session_id.clone(),
                            connected_at: c.connected_at.elapsed().as_secs(),
                            last_heartbeat: c.last_heartbeat.elapsed().as_secs(),
                        })
                        .collect()
                };

                LocalResponse::ClientList {
                    clients: client_infos,
                }
            }

            LocalCommand::SyncFile { file, destination } => {
                log::info!("Sync file request: {} -> {}", file, destination);

                match Self::sync_file_to_clients(&file, &destination, registry, rsync_storage).await {
                    Ok(count) => LocalResponse::Success {
                        message: format!("Synced {} to {} clients", file, count),
                    },
                    Err(e) => LocalResponse::Error {
                        message: format!("Failed to sync file: {:#}", e),
                    },
                }
            }

            LocalCommand::Execute {
                target,
                binary,
                args,
            } => {
                log::info!("Execute request: {} on {}", binary, target);

                let request_id = format!("exec-{}", uuid::Uuid::new_v4());
                let exec_msg = ServerMessage::Execute {
                    request_id: request_id.clone(),
                    binary,
                    args,
                    working_dir: None,
                    env: std::collections::HashMap::new(),
                };

                let result = registry
                    .lock()
                    .await
                    .send_to_client(&target, &exec_msg)
                    .await;

                match result {
                    Ok(_) => LocalResponse::Success {
                        message: format!("Execute command sent to {}", target),
                    },
                    Err(e) => LocalResponse::Error {
                        message: format!("Failed to send execute command: {:#}", e),
                    },
                }
            }

            LocalCommand::Shutdown => {
                log::info!("Shutdown request received");

                // Send shutdown message to all connected clients
                let client_count = {
                    let reg = registry.lock().await;
                    reg.client_count()
                };

                if client_count > 0 {
                    log::info!("Sending shutdown notification to {} clients", client_count);
                    let shutdown_msg = ServerMessage::Shutdown {
                        message: Some("Server is shutting down".to_string()),
                    };
                    let _ = registry.lock().await.broadcast(&shutdown_msg).await;

                    // Give clients a moment to receive the message
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }

                // Exit the process
                log::info!("Server shutting down");
                std::process::exit(0);
            }

            LocalCommand::Status => {
                log::info!("Status request received");

                let hostname = hostname::get()
                    .unwrap_or_else(|_| "unknown".into())
                    .to_string_lossy()
                    .to_string();

                let uptime = start_time.elapsed().as_secs();

                let client_infos: Vec<halfremembered_protocol::ClientInfo> = {
                    let reg = registry.lock().await;
                    let clients = reg.list_clients();
                    clients
                        .iter()
                        .map(|c| halfremembered_protocol::ClientInfo {
                            hostname: c.hostname.clone(),
                            platform: c.platform.clone(),
                            session_id: c.session_id.clone(),
                            connected_at: c.connected_at.elapsed().as_secs(),
                            last_heartbeat: c.last_heartbeat.elapsed().as_secs(),
                        })
                        .collect()
                };

                LocalResponse::Status {
                    hostname,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime,
                    clients: client_infos,
                }
            }

            LocalCommand::WatchDirectory {
                path,
                recursive,
                include_patterns,
                exclude_patterns,
            } => {
                log::info!("Watch directory request: {} (recursive: {})", path, recursive);
                log::debug!("Include patterns: {:?}", include_patterns);
                log::debug!("Exclude patterns: {:?}", exclude_patterns);

                let mut watcher_lock = file_watcher.lock().await;

                // Create FileWatcher lazily on first watch
                if watcher_lock.is_none() {
                    let registry_clone = registry.clone();
                    let storage_clone = rsync_storage.clone();

                    // Create callback that syncs files when they change
                    let callback = move |_watch_root: PathBuf, relative: PathBuf, absolute: PathBuf| {
                        let registry = registry_clone.clone();
                        let storage = storage_clone.clone();
                        let relative_str = relative.to_string_lossy().to_string();

                        log::info!("File changed: {} -> syncing to clients", absolute.display());

                        tokio::spawn(async move {
                            if let Err(e) = Self::sync_file_to_clients(
                                &absolute.to_string_lossy(),
                                &relative_str,
                                registry,
                                storage,
                            ).await {
                                log::error!("Failed to sync changed file: {:#}", e);
                            }
                        });
                    };

                    match FileWatcher::new(callback) {
                        Ok(watcher) => {
                            log::info!("Created FileWatcher");
                            *watcher_lock = Some(watcher);
                        }
                        Err(e) => {
                            return LocalResponse::Error {
                                message: format!("Failed to create file watcher: {:#}", e),
                            };
                        }
                    }
                }

                // Add the watch
                let result = watcher_lock
                    .as_mut()
                    .unwrap()
                    .add_watch(
                        PathBuf::from(&path),
                        recursive,
                        include_patterns,
                        exclude_patterns,
                    );

                match result {
                    Ok(_) => LocalResponse::Success {
                        message: format!("Watching {}", path),
                    },
                    Err(e) => LocalResponse::Error {
                        message: format!("Failed to add watch: {:#}", e),
                    },
                }
            }

            LocalCommand::UnwatchDirectory { path } => {
                log::info!("Unwatch directory request: {}", path);

                let mut watcher_lock = file_watcher.lock().await;

                if let Some(watcher) = watcher_lock.as_mut() {
                    match watcher.remove_watch(Path::new(&path)) {
                        Ok(_) => LocalResponse::Success {
                            message: format!("Stopped watching {}", path),
                        },
                        Err(e) => LocalResponse::Error {
                            message: format!("Failed to remove watch: {:#}", e),
                        },
                    }
                } else {
                    LocalResponse::Error {
                        message: "No file watcher active".to_string(),
                    }
                }
            }

            LocalCommand::ListWatches => {
                log::info!("List watches request");

                let watcher_lock = file_watcher.lock().await;

                if let Some(watcher) = watcher_lock.as_ref() {
                    LocalResponse::WatchList {
                        watches: watcher.list_watches(),
                    }
                } else {
                    LocalResponse::WatchList { watches: vec![] }
                }
            }
        }
    }

    async fn sync_file_to_clients(
        file_path: &str,
        destination: &str,
        registry: Arc<Mutex<ClientRegistry>>,
        rsync_storage: RsyncFileStorage,
    ) -> Result<usize> {
        let path = Path::new(file_path);

        if !path.exists() {
            anyhow::bail!("File not found: {}", file_path);
        }

        // Read file data
        let file_data = tokio::fs::read(&path)
            .await
            .context("Failed to read file")?;

        // Read file and compute metadata
        let metadata = tokio::fs::metadata(&path)
            .await
            .context("Failed to read file metadata")?;

        let size = metadata.len();
        let mtime = metadata
            .modified()
            .context("Failed to get file mtime")?
            .duration_since(std::time::UNIX_EPOCH)
            .context("Invalid mtime")?
            .as_secs();

        // Compute checksum
        let checksum = rsync_utils::compute_checksum(&file_data);

        // Choose block size
        let block_size = rsync_utils::choose_block_size(size);

        log::info!(
            "Syncing {} ({} bytes, checksum: {}, block_size: {}) to all clients via rsync",
            file_path,
            size,
            &checksum[..8],
            block_size
        );

        // Broadcast to all clients
        let request_id = format!("rsync-{}", uuid::Uuid::new_v4());
        let rsync_msg = ServerMessage::RsyncStart {
            request_id: request_id.clone(),
            relative_path: destination.to_string(),
            size,
            checksum,
            mtime,
            block_size,
        };

        let client_count = {
            let reg = registry.lock().await;
            reg.client_count()
        };

        if client_count == 0 {
            log::warn!("No clients connected to sync to");
            return Ok(0);
        }

        // Store file data for rsync operations
        rsync_storage
            .lock()
            .await
            .insert(request_id.clone(), (path.to_path_buf(), file_data));

        registry.lock().await.broadcast(&rsync_msg).await?;

        log::info!("Broadcast rsync start to {} clients", client_count);
        Ok(client_count)
    }

    #[allow(dead_code)]
    pub fn get_registry(&self) -> Arc<Mutex<ClientRegistry>> {
        self.client_registry.clone()
    }
}

impl russh::server::Server for SshServer {
    type Handler = SshSession;

    fn new_client(&mut self, addr: Option<SocketAddr>) -> Self::Handler {
        let session_id = Uuid::new_v4().to_string();
        log::info!(
            "New client connection from {:?}, session: {}",
            addr,
            session_id
        );

        SshSession {
            client_registry: self.client_registry.clone(),
            authorized_keys: self.authorized_keys.clone(),
            session_id,
            hostname: None,
            control_channel_id: None,
            message_buffer: MessageBuffer::new(),
            session_type: SessionType::Unknown,
            rsync_channels: HashMap::new(),
            rsync_file_storage: self.rsync_file_storage.clone(),
            file_watcher: self.file_watcher.clone(),
            start_time: self.start_time.clone(),
        }
    }
}

enum SessionType {
    Unknown,
    ClientDaemon,
    ControlCommand,
}

struct RsyncChannelState {
    request_id: Option<String>,
    file_path: Option<PathBuf>,
    file_data: Option<Vec<u8>>,
    frame_buffer: FrameBuffer,
}

impl RsyncChannelState {
    fn new() -> Self {
        Self {
            request_id: None,
            file_path: None,
            file_data: None,
            frame_buffer: FrameBuffer::new(),
        }
    }
}

pub struct SshSession {
    client_registry: Arc<Mutex<ClientRegistry>>,
    authorized_keys: Arc<Vec<ssh_key::PublicKey>>,
    session_id: String,
    hostname: Option<String>,
    control_channel_id: Option<ChannelId>,
    message_buffer: MessageBuffer,
    session_type: SessionType,
    rsync_channels: HashMap<ChannelId, RsyncChannelState>,
    rsync_file_storage: RsyncFileStorage,
    file_watcher: FileWatcherRef,
    start_time: Arc<Instant>,
}

impl russh::server::Handler for SshSession {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        let client_fingerprint = public_key.fingerprint(ssh_key::HashAlg::Sha256);
        log::debug!(
            "Public key auth attempt for user: {} with key: {}",
            user,
            client_fingerprint
        );

        // Use fingerprint comparison instead of PartialEq
        // NOTE: ssh_key crate's PartialEq incorrectly includes the comment field,
        // which differs between ssh-agent (empty comment) and authorized_keys (has comment).
        // Fingerprints are the correct way to compare SSH keys - they hash only the
        // algorithm and public key bytes, not metadata like comments.
        for authorized_key in self.authorized_keys.iter() {
            let auth_fingerprint = authorized_key.fingerprint(ssh_key::HashAlg::Sha256);

            if client_fingerprint == auth_fingerprint {
                log::info!("✓ Public key authentication successful for {}", user);
                return Ok(Auth::Accept);
            }
        }

        log::warn!(
            "Public key authentication failed for {}: key not in authorized_keys",
            user
        );
        log::debug!("Client fingerprint: {}", client_fingerprint);
        Ok(Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let channel_id = channel.id();
        log::debug!("Session channel opened: {:?}", channel_id);

        // First channel is the control channel
        if self.control_channel_id.is_none() {
            log::debug!("Setting control channel: {:?}", channel_id);
            self.control_channel_id = Some(channel_id);
        } else {
            // Additional channels are rsync channels
            log::debug!("Detected rsync channel: {:?}", channel_id);
            self.rsync_channels.insert(channel_id, RsyncChannelState::new());
        }

        Ok(true)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Check if this is an rsync channel
        if self.rsync_channels.contains_key(&channel) {
            return self.handle_rsync_data(channel, data, session).await;
        }

        // Control channel data
        self.message_buffer.append(data);

        match self.session_type {
            SessionType::Unknown => {
                // Try to determine session type from first message
                if let Some(msg) = self
                    .message_buffer
                    .try_parse_client_message()
                    .map_err(|e| russh::Error::from(std::io::Error::other(e)))?
                {
                    log::debug!("Detected client daemon session");
                    self.session_type = SessionType::ClientDaemon;
                    self.handle_client_message(msg, channel, session).await?;
                } else if let Some(cmd) = self
                    .message_buffer
                    .try_parse_local_command()
                    .map_err(|e| russh::Error::from(std::io::Error::other(e)))?
                {
                    log::debug!("Detected control command session");
                    self.session_type = SessionType::ControlCommand;
                    self.handle_control_command(cmd, channel, session).await?;
                }
            }
            SessionType::ClientDaemon => {
                while let Some(msg) = self
                    .message_buffer
                    .try_parse_client_message()
                    .map_err(|e| russh::Error::from(std::io::Error::other(e)))?
                {
                    self.handle_client_message(msg, channel, session).await?;
                }
            }
            SessionType::ControlCommand => {
                while let Some(cmd) = self
                    .message_buffer
                    .try_parse_local_command()
                    .map_err(|e| russh::Error::from(std::io::Error::other(e)))?
                {
                    self.handle_control_command(cmd, channel, session).await?;
                }
            }
        }

        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        log::debug!("Channel EOF: {:?}", channel);
        Ok(())
    }
}

impl SshSession {
    async fn handle_client_message(
        &mut self,
        msg: ClientMessage,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        log::debug!("Received {}", msg.message_type());

        match msg {
            ClientMessage::Register { hostname, platform } => {
                log::info!("Client registered: {} ({})", hostname, platform);

                self.hostname = Some(hostname.clone());

                let client = ConnectedClient {
                    hostname: hostname.clone(),
                    session_id: self.session_id.clone(),
                    platform,
                    connected_at: Instant::now(),
                    last_heartbeat: Instant::now(),
                    session_handle: session.handle(),
                    channel_id: channel,
                };

                self.client_registry
                    .lock()
                    .await
                    .register(client)
                    .map_err(|e| russh::Error::from(std::io::Error::other(e)))?;

                let welcome = ServerMessage::Welcome {
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
                    session_id: self.session_id.clone(),
                };

                self.send_message(&welcome, channel, session).await?;

                // Send a test ping to verify bidirectional communication
                let ping = ServerMessage::Ping {
                    request_id: format!("test-ping-{}", self.session_id),
                };

                log::info!("Sending test ping to {}", hostname);
                self.send_message(&ping, channel, session).await?;
            }

            ClientMessage::Heartbeat {
                timestamp: _,
                sequence,
            } => {
                log::trace!("Heartbeat from {:?}: seq={}", self.hostname, sequence);

                if let Some(ref hostname) = self.hostname {
                    self.client_registry.lock().await.update_heartbeat(hostname);
                }
            }

            ClientMessage::RsyncComplete {
                request_id,
                path,
                success,
                checksum,
                bytes_transferred,
                error,
            } => {
                if success {
                    log::info!(
                        "Rsync complete: {} ({} bytes transferred, checksum: {}, request: {})",
                        path,
                        bytes_transferred,
                        &checksum[..8],
                        request_id
                    );
                } else {
                    log::error!(
                        "Rsync failed: {} (request: {}, error: {:?})",
                        path,
                        request_id,
                        error
                    );
                }
            }

            ClientMessage::ExecComplete {
                request_id,
                exit_code,
                stdout,
                stderr,
            } => {
                log::info!(
                    "Execution complete (request: {}, exit: {})",
                    request_id,
                    exit_code
                );
                if !stdout.is_empty() {
                    log::debug!("stdout: {}", stdout);
                }
                if !stderr.is_empty() {
                    log::debug!("stderr: {}", stderr);
                }
            }

            ClientMessage::Status { request_id, state } => {
                log::info!("Status (request: {}): {:?}", request_id, state);
            }

            ClientMessage::Error {
                request_id,
                message,
            } => {
                log::error!("Client error (request: {:?}): {}", request_id, message);
            }
        }

        Ok(())
    }

    async fn send_message(
        &self,
        msg: &ServerMessage,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let mut full_message = Vec::new();
        msg.write_framed(&mut full_message)
            .map_err(|e| russh::Error::from(std::io::Error::other(e)))?;

        let _ = session.data(channel, full_message.into());

        log::debug!("Sent {}", msg.message_type());
        Ok(())
    }

    async fn handle_control_command(
        &self,
        command: LocalCommand,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        log::debug!("Handling control command: {:?}", command);

        let response = SshServer::handle_local_command(
            command,
            self.client_registry.clone(),
            self.rsync_file_storage.clone(),
            self.file_watcher.clone(),
            self.start_time.clone(),
        )
        .await;

        let mut full_message = Vec::new();
        response
            .write_framed(&mut full_message)
            .map_err(|e| russh::Error::from(std::io::Error::other(e)))?;

        let _ = session.data(channel, full_message.into());

        log::debug!("Sent LocalResponse");
        Ok(())
    }

    async fn handle_rsync_data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        // Check if channel exists
        if !self.rsync_channels.contains_key(&channel) {
            return Err(russh::Error::from(std::io::Error::other(
                "Rsync channel not found",
            )));
        }

        // Append data to frame buffer
        self.rsync_channels
            .get_mut(&channel)
            .unwrap()
            .frame_buffer
            .append(data);

        // Try to parse frames
        let mut should_remove_channel = false;
        loop {
            let frame = {
                let state = self.rsync_channels.get_mut(&channel).unwrap();
                match state.frame_buffer.try_parse() {
                    Ok(Some(f)) => f,
                    Ok(None) => break, // Need more data
                    Err(e) => {
                        log::error!("Frame parse error on rsync channel: {:#}", e);
                        return Err(russh::Error::from(std::io::Error::other(e)));
                    }
                }
            };

            log::debug!(
                "Received rsync frame on channel {:?}: type={}, size={}",
                channel,
                frame.message_type,
                frame.payload.len()
            );

            // Handle frame based on type
            match frame.message_type {
                MSG_RSYNC_SIGNATURE => {
                    let state = self.rsync_channels.get_mut(&channel).unwrap();

                    // First frame is handshake with request_id
                    if state.request_id.is_none() {
                        let request_id =
                            String::from_utf8(frame.payload).map_err(|e| {
                                russh::Error::from(std::io::Error::other(format!(
                                    "Invalid request_id: {}",
                                    e
                                )))
                            })?;

                        log::debug!("Rsync handshake: request_id={}", request_id);
                        state.request_id = Some(request_id.clone());

                        // Look up file data for this request
                        let files = self.rsync_file_storage.lock().await;
                        if let Some((file_path, file_data)) = files.get(&request_id) {
                            state.file_path = Some(file_path.clone());
                            state.file_data = Some(file_data.clone());
                            log::debug!("Found file for request: {} bytes", file_data.len());
                        } else {
                            log::error!("No file found for request_id: {}", request_id);
                            return Err(russh::Error::from(std::io::Error::other(format!(
                                "No file found for request_id: {}",
                                request_id
                            ))));
                        }
                    } else {
                        // Second frame is signature data
                        log::debug!("Received signature: {} bytes", frame.payload.len());

                        // Generate delta
                        let file_data = state
                            .file_data
                            .as_ref()
                            .ok_or_else(|| {
                                russh::Error::from(std::io::Error::other("No file data"))
                            })?
                            .clone();

                        let delta = rsync_utils::generate_delta(&file_data, &frame.payload)
                            .map_err(|e| {
                                russh::Error::from(std::io::Error::other(format!(
                                    "Failed to generate delta: {:#}",
                                    e
                                )))
                            })?;

                        log::debug!("Generated delta: {} bytes", delta.len());

                        // Send delta frame
                        let delta_frame = Frame::new(MSG_RSYNC_DELTA, delta);
                        let mut buffer = Vec::new();
                        delta_frame.write(&mut buffer).map_err(|e| {
                            russh::Error::from(std::io::Error::other(format!(
                                "Failed to write frame: {:#}",
                                e
                            )))
                        })?;

                        let _ = session.data(channel, buffer.into());
                        log::debug!("Sent delta frame on channel {:?}", channel);

                        // Close the rsync channel
                        let _ = session.eof(channel);
                        log::debug!("Closed rsync channel {:?}", channel);

                        // Mark for removal
                        should_remove_channel = true;
                    }
                }
                _ => {
                    log::warn!("Unexpected frame type on rsync channel: {}", frame.message_type);
                }
            }
        }

        // Clean up channel if needed
        if should_remove_channel {
            self.rsync_channels.remove(&channel);
        }

        Ok(())
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        if let Some(ref hostname) = self.hostname {
            let registry = self.client_registry.clone();
            let hostname = hostname.clone();
            tokio::spawn(async move {
                registry.lock().await.unregister(&hostname);
            });
        }
    }
}
