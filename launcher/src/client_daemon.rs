use anyhow::{Context, Result};
use halfremembered_protocol::{
    ClientMessage, ClientState, Frame, ServerMessage, MSG_RSYNC_DELTA, MSG_RSYNC_SIGNATURE,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;

use crate::rsync_utils;
use crate::ssh_client::SshClientConnection;

pub struct ClientDaemon {
    server_host: String,
    server_port: u16,
    server_user: String,
    hostname: String,
    heartbeat_interval: Duration,
    reconnect_delay: Duration,
    agent_socket: Option<String>,
    working_dir: Option<std::path::PathBuf>,
    initial_sync: bool,
    shutdown: Arc<AtomicBool>,
    state: Arc<Mutex<ClientState>>,
    connection: Option<SshClientConnection>,
}

impl ClientDaemon {
    pub fn new(
        server_host: String,
        server_port: u16,
        server_user: String,
        hostname: String,
    ) -> Self {
        let connected_since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            server_host,
            server_port,
            server_user,
            hostname,
            heartbeat_interval: Duration::from_secs(30),
            reconnect_delay: Duration::from_secs(5),
            agent_socket: None,
            working_dir: None,
            initial_sync: true,
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(ClientState {
                connected_since,
                last_sync: None,
                running_processes: Vec::new(),
                pending_transfers: 0,
            })),
            connection: None,
        }
    }

    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    pub fn with_reconnect_delay(mut self, delay: Duration) -> Self {
        self.reconnect_delay = delay;
        self
    }

    pub fn with_agent_socket(mut self, agent_socket: Option<String>) -> Self {
        self.agent_socket = agent_socket;
        self
    }

    pub fn with_working_dir(mut self, working_dir: std::path::PathBuf) -> Self {
        self.working_dir = Some(working_dir);
        self
    }

    pub fn with_initial_sync(mut self, initial_sync: bool) -> Self {
        self.initial_sync = initial_sync;
        self
    }

    pub async fn run(&mut self) -> Result<()> {
        log::info!("Starting client daemon for {}", self.hostname);

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                log::info!("Shutdown requested, exiting");
                break;
            }

            match self.connect_and_run().await {
                Ok(_) => {
                    log::info!("Control loop exited normally");
                    break;
                }
                Err(e) => {
                    log::error!("Connection error: {:#}", e);
                    log::info!(
                        "Reconnecting in {} seconds...",
                        self.reconnect_delay.as_secs()
                    );
                    time::sleep(self.reconnect_delay).await;

                    self.reconnect_delay =
                        std::cmp::min(self.reconnect_delay * 2, Duration::from_secs(60));
                }
            }
        }

        Ok(())
    }

    async fn connect_and_run(&mut self) -> Result<()> {
        log::info!(
            "Connecting to {}@{}:{}",
            self.server_user,
            self.server_host,
            self.server_port
        );

        let connection = SshClientConnection::connect(
            &self.server_host,
            self.server_port,
            &self.server_user,
            self.agent_socket.as_deref(),
        )
        .await?;

        connection
            .send_register(&self.hostname, self.initial_sync)
            .await
            .context("Failed to send registration")?;

        log::info!("Sent registration message");

        self.connection = Some(connection);
        self.reconnect_delay = Duration::from_secs(5);

        self.control_loop().await
    }

    async fn control_loop(&mut self) -> Result<()> {
        let mut heartbeat_timer = time::interval(self.heartbeat_interval);
        heartbeat_timer.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        log::info!("Entering control loop");

        loop {
            tokio::select! {
                _ = heartbeat_timer.tick() => {
                    self.handle_heartbeat().await?;
                }

                _ = time::sleep(Duration::from_millis(100)) => {
                    if let Some(msg) = self.poll_server_message().await? {
                        self.handle_server_message(msg).await?;
                    }

                    if self.shutdown.load(Ordering::Relaxed) {
                        log::info!("Shutdown requested in control loop");
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn poll_server_message(&mut self) -> Result<Option<ServerMessage>> {
        if let Some(ref conn) = self.connection {
            conn.try_receive_message().await
        } else {
            Ok(None)
        }
    }

    async fn handle_heartbeat(&mut self) -> Result<()> {
        if let Some(ref conn) = self.connection {
            conn.send_heartbeat(0)
                .await
                .context("Failed to send heartbeat")?;
            log::trace!("Sent heartbeat");
        }
        Ok(())
    }

    async fn handle_server_message(&mut self, msg: ServerMessage) -> Result<()> {
        log::debug!("Received server message: {:?}", msg);

        match msg {
            ServerMessage::Welcome {
                server_version,
                session_id,
            } => {
                log::info!(
                    "Server welcomed us: version={}, session={}",
                    server_version,
                    session_id
                );
            }

            ServerMessage::Ping { request_id } => {
                log::debug!("Received ping: {}", request_id);
                if let Some(ref conn) = self.connection {
                    let state = self.state.lock().unwrap().clone();
                    let msg = ClientMessage::Status { request_id, state };
                    conn.send_message(&msg).await?;
                }
            }

            ServerMessage::RsyncStart {
                request_id,
                relative_path,
                size,
                checksum,
                mtime,
                block_size,
                mode,
            } => {
                log::info!(
                    "Rsync request: {} ({} bytes, block_size: {})",
                    relative_path,
                    size,
                    block_size
                );
                self.handle_rsync_start(
                    request_id,
                    relative_path,
                    size,
                    checksum,
                    mtime,
                    block_size,
                    mode,
                )
                .await?;
            }

            ServerMessage::Execute {
                request_id,
                binary,
                args,
                working_dir,
                env,
            } => {
                log::info!("Execute request: {} {:?}", binary, args);
                self.handle_execute(request_id, binary, args, working_dir, env)
                    .await?;
            }

            ServerMessage::Shutdown { message } => {
                if let Some(msg) = message {
                    log::info!("Server requested shutdown: {}", msg);
                } else {
                    log::info!("Server requested shutdown");
                }
                self.shutdown.store(true, Ordering::Relaxed);
            }
        }

        Ok(())
    }

    async fn handle_rsync_start(
        &mut self,
        request_id: String,
        relative_path: String,
        _size: u64,
        expected_checksum: String,
        _mtime: u64,
        block_size: u32,
        mode: u32,
    ) -> Result<()> {
        log::info!("Rsync start: {} (block_size: {})", relative_path, block_size);

        let start_time = std::time::Instant::now();

        // Construct local path relative to working directory if set
        let local_path = if let Some(ref working_dir) = self.working_dir {
            working_dir.join(&relative_path)
        } else {
            std::path::PathBuf::from(&relative_path)
        };

        // Create parent directory if needed
        if let Some(parent) = local_path.parent()
            && !parent.exists()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create parent directory")?;
        }

        // Spawn rsync task
        let conn_ref = self
            .connection
            .as_ref()
            .context("No active connection")?;

        log::debug!("Opening rsync channel for {}", relative_path);

        // Open dedicated rsync channel
        let mut rsync_channel = conn_ref
            .open_rsync_channel()
            .await
            .context("Failed to open rsync channel")?;

        log::debug!("Successfully opened rsync channel for {}", relative_path);

        // Send request_id as handshake
        let handshake_frame = Frame::new(MSG_RSYNC_SIGNATURE, request_id.clone().into_bytes());
        SshClientConnection::write_frame_to_channel(&mut rsync_channel, &handshake_frame)
            .await
            .context("Failed to send handshake")?;

        log::debug!("Sent handshake for {}", relative_path);

        // Generate signature from local file (if it exists)
        let signature_data = if local_path.exists() {
            log::debug!(
                "Generating signature for existing file: {}",
                local_path.display()
            );
            rsync_utils::generate_signature(&local_path, block_size)
                .await
                .context("Failed to generate signature")?
        } else {
            log::debug!("No existing file, sending empty signature");
            Vec::new()
        };

        log::debug!("Signature size: {} bytes", signature_data.len());

        // Send signature on rsync channel
        let sig_frame = Frame::new(MSG_RSYNC_SIGNATURE, signature_data);
        SshClientConnection::write_frame_to_channel(&mut rsync_channel, &sig_frame)
            .await
            .context("Failed to send signature")?;

        log::debug!("Sent signature for {}", relative_path);

        // Receive delta on rsync channel (may be multiple chunks for large files)
        let mut delta_data = Vec::new();
        let mut chunk_count = 0;
        loop {
            let delta_frame = SshClientConnection::read_frame_from_channel(&mut rsync_channel)
                .await
                .context("Failed to receive delta chunk")?;

            if delta_frame.message_type != MSG_RSYNC_DELTA {
                anyhow::bail!(
                    "Expected delta frame, got message type: {}",
                    delta_frame.message_type
                );
            }

            // Zero-length frame signals end of delta stream
            if delta_frame.payload.is_empty() {
                log::debug!("Received end-of-delta marker after {} chunks, {} bytes total",
                    chunk_count, delta_data.len());
                break;
            }

            delta_data.extend_from_slice(&delta_frame.payload);
            chunk_count += 1;
            log::trace!("Received delta chunk {}: {} bytes (total: {} bytes)",
                chunk_count, delta_frame.payload.len(), delta_data.len());
        }

        let delta_size = delta_data.len();
        log::debug!("Received delta: {} bytes total in {} chunks", delta_size, chunk_count);

        // Apply delta to produce new file
        let base_path = if local_path.exists() {
            Some(local_path.as_path())
        } else {
            None
        };

        let new_content = rsync_utils::apply_delta(base_path, &delta_data)
            .await
            .context("Failed to apply delta")?;

        // Verify checksum
        let actual_checksum = rsync_utils::compute_checksum(&new_content);
        let success = actual_checksum == expected_checksum;

        if success {
            log::debug!("Checksum verified for {}", relative_path);

            // Write the file
            tokio::fs::write(&local_path, &new_content)
                .await
                .context("Failed to write file")?;

            // Apply file permissions from server
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let permissions = std::fs::Permissions::from_mode(mode);
                tokio::fs::set_permissions(&local_path, permissions)
                    .await
                    .context("Failed to set file permissions")?;
                log::debug!("Set permissions {:o} on {}", mode, relative_path);
            }
            #[cfg(not(unix))]
            {
                // Windows doesn't use Unix permissions, so we just log it
                log::trace!("Ignoring Unix permissions {:o} on Windows", mode);
            }

            let elapsed = start_time.elapsed();
            log::info!(
                "Successfully synced {} ({} bytes transferred in {:.2}s)",
                relative_path,
                delta_size,
                elapsed.as_secs_f64()
            );
        } else {
            log::error!(
                "Checksum mismatch for {}: expected {}, got {}",
                relative_path,
                expected_checksum,
                actual_checksum
            );
        }

        // Close rsync channel
        drop(rsync_channel);

        // Send RsyncComplete message on control channel
        let msg = ClientMessage::RsyncComplete {
            request_id,
            path: relative_path,
            success,
            checksum: actual_checksum,
            bytes_transferred: delta_size as u64,
            error: if success {
                None
            } else {
                Some("Checksum mismatch".to_string())
            },
        };

        if let Some(ref conn) = self.connection {
            conn.send_message(&msg).await?;
        }

        Ok(())
    }

    async fn handle_execute(
        &mut self,
        request_id: String,
        binary: String,
        args: Vec<String>,
        working_dir: Option<String>,
        env: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        log::info!("Executing: {} {:?}", binary, args);

        let result = self
            .execute_command(&binary, &args, working_dir.as_deref(), &env)
            .await;

        if let Some(ref conn) = self.connection {
            let (exit_code, stdout, stderr) = match result {
                Ok((code, out, err)) => (code, out, err),
                Err(e) => {
                    log::error!("Failed to execute {}: {:#}", binary, e);
                    (-1, String::new(), format!("Execution failed: {:#}", e))
                }
            };

            let msg = ClientMessage::ExecComplete {
                request_id,
                exit_code,
                stdout,
                stderr,
            };
            conn.send_message(&msg).await?;
        }

        Ok(())
    }

    async fn execute_command(
        &self,
        binary: &str,
        args: &[String],
        working_dir: Option<&str>,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<(i32, String, String)> {
        let mut command = tokio::process::Command::new(binary);
        command.args(args);

        // Set working directory if provided
        if let Some(dir) = working_dir {
            command.current_dir(dir);
        }

        // Add environment variables
        for (key, value) in env {
            command.env(key, value);
        }

        // Capture output
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        log::info!("Spawning process: {} {:?}", binary, args);

        let output = command
            .output()
            .await
            .context(format!("Failed to spawn process: {}", binary))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        log::info!(
            "Process completed: {} (exit: {}, stdout: {} bytes, stderr: {} bytes)",
            binary,
            exit_code,
            stdout.len(),
            stderr.len()
        );

        Ok((exit_code, stdout, stderr))
    }
}
