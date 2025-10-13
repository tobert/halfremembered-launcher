use anyhow::{Context, Result};
use halfremembered_protocol::ServerMessage;
use russh::server::Handle;
use russh::ChannelId;
use std::collections::HashMap;
use std::time::Instant;

pub struct ClientRegistry {
    clients: HashMap<String, ConnectedClient>,
}

pub struct ConnectedClient {
    pub hostname: String,
    pub session_id: String,
    pub platform: String,
    pub connected_at: Instant,
    pub last_heartbeat: Instant,
    pub session_handle: Handle,
    pub channel_id: ChannelId,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn register(&mut self, client: ConnectedClient) -> Result<()> {
        log::info!(
            "Registering client: {} (session: {}, platform: {})",
            client.hostname,
            client.session_id,
            client.platform
        );
        self.clients.insert(client.hostname.clone(), client);
        Ok(())
    }

    pub fn unregister(&mut self, hostname: &str) {
        if self.clients.remove(hostname).is_some() {
            log::info!("Unregistered client: {}", hostname);
        }
    }

    pub async fn send_to_client(&mut self, hostname: &str, msg: &ServerMessage) -> Result<()> {
        let client = self
            .clients
            .get(hostname)
            .context(format!("Client not found: {}", hostname))?;

        let mut full_message = Vec::new();
        msg.write_framed(&mut full_message)
            .context("Failed to serialize server message")?;

        client
            .session_handle
            .data(client.channel_id, full_message.into())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send message to client: {:?}", e))?;

        log::debug!("Sent {} to {}", msg.message_type(), hostname);
        Ok(())
    }

    pub async fn broadcast(&mut self, msg: &ServerMessage) -> Result<()> {
        let mut full_message = Vec::new();
        msg.write_framed(&mut full_message)
            .context("Failed to serialize server message")?;

        for (hostname, client) in &self.clients {
            if let Err(e) = client
                .session_handle
                .data(client.channel_id, full_message.clone().into())
                .await
            {
                log::error!("Failed to broadcast to {}: {:?}", hostname, e);
            } else {
                log::debug!("Broadcast {} to {}", msg.message_type(), hostname);
            }
        }

        Ok(())
    }

    pub fn update_heartbeat(&mut self, hostname: &str) {
        if let Some(client) = self.clients.get_mut(hostname) {
            client.last_heartbeat = Instant::now();
        }
    }

    pub fn list_clients(&self) -> Vec<&ConnectedClient> {
        self.clients.values().collect()
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}
