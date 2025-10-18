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
        self.clients.insert(client.session_id.clone(), client);
        Ok(())
    }

    pub fn unregister(&mut self, session_id: &str) {
        if self.clients.remove(session_id).is_some() {
            log::info!("Unregistered client session: {}", session_id);
        }
    }

    pub async fn send_to_client(&mut self, hostname: &str, msg: &ServerMessage) -> Result<()> {
        let client = self
            .clients
            .values()
            .find(|c| c.hostname == hostname)
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
        if let Some(client) = self.clients.values_mut().find(|c| c.hostname == hostname) {
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
