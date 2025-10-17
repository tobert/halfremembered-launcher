use anyhow::{Context, Result};
use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};

// Unified frame protocol
pub mod frame;
pub mod message_types;

// Re-export commonly used types
pub use frame::{Frame, FrameBuffer, FRAME_HEADER_SIZE, MAX_FRAME_SIZE};
pub use message_types::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientMessage {
    Register {
        hostname: String,
        platform: String,
    },
    Heartbeat {
        timestamp: u64,
        sequence: u32,
    },
    RsyncComplete {
        request_id: String,
        path: String,
        success: bool,
        checksum: String,
        bytes_transferred: u64,
        error: Option<String>,
    },
    ExecComplete {
        request_id: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    Status {
        request_id: String,
        state: ClientState,
    },
    Error {
        request_id: Option<String>,
        message: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    Welcome {
        server_version: String,
        session_id: String,
    },
    RsyncStart {
        request_id: String,
        relative_path: String,
        size: u64,
        checksum: String,
        mtime: u64,
        block_size: u32,
    },
    Execute {
        request_id: String,
        binary: String,
        args: Vec<String>,
        working_dir: Option<String>,
        env: HashMap<String, String>,
    },
    Ping {
        request_id: String,
    },
    Shutdown {
        message: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientState {
    pub connected_since: u64,
    pub last_sync: Option<u64>,
    pub running_processes: Vec<String>,
    pub pending_transfers: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LocalCommand {
    Status,
    Ping {
        target: String,
    },
    ListClients,
    Shutdown,
    SyncFile {
        file: String,
        destination: String,
    },
    Execute {
        target: String,
        binary: String,
        args: Vec<String>,
    },
    WatchDirectory {
        path: String,
        recursive: bool,
        include_patterns: Vec<String>,
        exclude_patterns: Vec<String>,
    },
    UnwatchDirectory {
        path: String,
    },
    ListWatches,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientInfo {
    pub hostname: String,
    pub platform: String,
    pub session_id: String,
    pub connected_at: u64,
    pub last_heartbeat: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WatchInfo {
    pub path: String,
    pub recursive: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LocalResponse {
    Success {
        message: String,
    },
    Error {
        message: String,
    },
    Status {
        hostname: String,
        version: String,
        uptime: u64,
        clients: Vec<ClientInfo>,
    },
    ClientList {
        clients: Vec<ClientInfo>,
    },
    WatchList {
        watches: Vec<WatchInfo>,
    },
}

// Rsync protocol messages

/// Server initiates file sync on control channel
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RsyncStart {
    pub request_id: String,
    pub relative_path: String,
    pub size: u64,
    pub checksum: String,
    pub mtime: u64,
    pub block_size: u32,
}

/// Client reports sync completion on control channel
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RsyncComplete {
    pub request_id: String,
    pub path: String,
    pub success: bool,
    pub checksum: String,
    pub bytes_transferred: u64,
    pub error: Option<String>,
}

impl LocalCommand {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).context("Failed to serialize LocalCommand")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).context("Failed to deserialize LocalCommand")
    }

    pub fn write_framed<W: Write>(&self, writer: &mut W) -> Result<()> {
        let bytes = self.to_bytes()?;
        let len = (bytes.len() + 1) as u32; // +1 for type byte

        if len as usize > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        writer
            .write_all(&len.to_be_bytes())
            .context("Failed to write message length")?;
        writer
            .write_all(&[MESSAGE_TYPE_LOCAL_COMMAND])
            .context("Failed to write message type")?;
        writer
            .write_all(&bytes)
            .context("Failed to write message body")?;
        writer.flush().context("Failed to flush writer")?;

        log::trace!("Sent LocalCommand: {} bytes", len);
        Ok(())
    }

    pub fn read_framed<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_bytes = [0u8; 4];
        reader
            .read_exact(&mut len_bytes)
            .context("Failed to read message length")?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        let mut type_byte = [0u8; 1];
        reader
            .read_exact(&mut type_byte)
            .context("Failed to read message type")?;

        if type_byte[0] != MESSAGE_TYPE_LOCAL_COMMAND {
            anyhow::bail!(
                "Invalid message type: expected {}, got {}",
                MESSAGE_TYPE_LOCAL_COMMAND,
                type_byte[0]
            );
        }

        let mut buf = vec![0u8; len - 1]; // -1 for type byte
        reader
            .read_exact(&mut buf)
            .context("Failed to read message body")?;

        let msg = Self::from_bytes(&buf)?;
        log::trace!("Received LocalCommand: {} bytes", len);
        Ok(msg)
    }
}

impl LocalResponse {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).context("Failed to serialize LocalResponse")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).context("Failed to deserialize LocalResponse")
    }

    pub fn write_framed<W: Write>(&self, writer: &mut W) -> Result<()> {
        let bytes = self.to_bytes()?;
        let len = (bytes.len() + 1) as u32; // +1 for type byte

        if len as usize > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        writer
            .write_all(&len.to_be_bytes())
            .context("Failed to write message length")?;
        writer
            .write_all(&[MESSAGE_TYPE_LOCAL_RESPONSE])
            .context("Failed to write message type")?;
        writer
            .write_all(&bytes)
            .context("Failed to write message body")?;
        writer.flush().context("Failed to flush writer")?;

        log::trace!("Sent LocalResponse: {} bytes", len);
        Ok(())
    }

    pub fn read_framed<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_bytes = [0u8; 4];
        reader
            .read_exact(&mut len_bytes)
            .context("Failed to read message length")?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        let mut type_byte = [0u8; 1];
        reader
            .read_exact(&mut type_byte)
            .context("Failed to read message type")?;

        if type_byte[0] != MESSAGE_TYPE_LOCAL_RESPONSE {
            anyhow::bail!(
                "Invalid message type: expected {}, got {}",
                MESSAGE_TYPE_LOCAL_RESPONSE,
                type_byte[0]
            );
        }

        let mut buf = vec![0u8; len - 1]; // -1 for type byte
        reader
            .read_exact(&mut buf)
            .context("Failed to read message body")?;

        let msg = Self::from_bytes(&buf)?;
        log::trace!("Received LocalResponse: {} bytes", len);
        Ok(msg)
    }
}

impl ClientMessage {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).context("Failed to serialize ClientMessage")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).context("Failed to deserialize ClientMessage")
    }

    pub fn write_framed<W: Write>(&self, writer: &mut W) -> Result<()> {
        let bytes = self.to_bytes()?;
        let len = (bytes.len() + 1) as u32; // +1 for type byte

        if len as usize > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        writer
            .write_all(&len.to_be_bytes())
            .context("Failed to write message length")?;
        writer
            .write_all(&[MESSAGE_TYPE_CLIENT])
            .context("Failed to write message type")?;
        writer
            .write_all(&bytes)
            .context("Failed to write message body")?;
        writer.flush().context("Failed to flush writer")?;

        log::trace!(
            "Sent ClientMessage: {} bytes, type: {:?}",
            len,
            self.message_type()
        );
        Ok(())
    }

    pub fn read_framed<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_bytes = [0u8; 4];
        reader
            .read_exact(&mut len_bytes)
            .context("Failed to read message length")?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        let mut type_byte = [0u8; 1];
        reader
            .read_exact(&mut type_byte)
            .context("Failed to read message type")?;

        if type_byte[0] != MESSAGE_TYPE_CLIENT {
            anyhow::bail!(
                "Invalid message type: expected {}, got {}",
                MESSAGE_TYPE_CLIENT,
                type_byte[0]
            );
        }

        let mut buf = vec![0u8; len - 1]; // -1 for type byte
        reader
            .read_exact(&mut buf)
            .context("Failed to read message body")?;

        let msg = Self::from_bytes(&buf)?;

        log::trace!(
            "Received ClientMessage: {} bytes, type: {:?}",
            len,
            msg.message_type()
        );
        Ok(msg)
    }

    pub fn message_type(&self) -> &'static str {
        match self {
            ClientMessage::Register { .. } => "Register",
            ClientMessage::Heartbeat { .. } => "Heartbeat",
            ClientMessage::RsyncComplete { .. } => "RsyncComplete",
            ClientMessage::ExecComplete { .. } => "ExecComplete",
            ClientMessage::Status { .. } => "Status",
            ClientMessage::Error { .. } => "Error",
        }
    }
}

impl ServerMessage {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).context("Failed to serialize ServerMessage")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bincode::deserialize(bytes).context("Failed to deserialize ServerMessage")
    }

    pub fn write_framed<W: Write>(&self, writer: &mut W) -> Result<()> {
        let bytes = self.to_bytes()?;
        let len = (bytes.len() + 1) as u32; // +1 for type byte

        if len as usize > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        writer
            .write_all(&len.to_be_bytes())
            .context("Failed to write message length")?;
        writer
            .write_all(&[MESSAGE_TYPE_SERVER])
            .context("Failed to write message type")?;
        writer
            .write_all(&bytes)
            .context("Failed to write message body")?;
        writer.flush().context("Failed to flush writer")?;

        log::trace!(
            "Sent ServerMessage: {} bytes, type: {:?}",
            len,
            self.message_type()
        );
        Ok(())
    }

    pub fn read_framed<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_bytes = [0u8; 4];
        reader
            .read_exact(&mut len_bytes)
            .context("Failed to read message length")?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "Message too large: {} bytes (max: {})",
                len,
                MAX_MESSAGE_SIZE
            );
        }

        let mut type_byte = [0u8; 1];
        reader
            .read_exact(&mut type_byte)
            .context("Failed to read message type")?;

        if type_byte[0] != MESSAGE_TYPE_SERVER {
            anyhow::bail!(
                "Invalid message type: expected {}, got {}",
                MESSAGE_TYPE_SERVER,
                type_byte[0]
            );
        }

        let mut buf = vec![0u8; len - 1]; // -1 for type byte
        reader
            .read_exact(&mut buf)
            .context("Failed to read message body")?;

        let msg = Self::from_bytes(&buf)?;

        log::trace!(
            "Received ServerMessage: {} bytes, type: {:?}",
            len,
            msg.message_type()
        );
        Ok(msg)
    }

    pub fn message_type(&self) -> &'static str {
        match self {
            ServerMessage::Welcome { .. } => "Welcome",
            ServerMessage::RsyncStart { .. } => "RsyncStart",
            ServerMessage::Execute { .. } => "Execute",
            ServerMessage::Ping { .. } => "Ping",
            ServerMessage::Shutdown { .. } => "Shutdown",
        }
    }
}

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

// Message type discriminators
const MESSAGE_TYPE_CLIENT: u8 = 0x01;
const MESSAGE_TYPE_SERVER: u8 = 0x02;
const MESSAGE_TYPE_LOCAL_COMMAND: u8 = 0x03;
const MESSAGE_TYPE_LOCAL_RESPONSE: u8 = 0x04;

pub struct MessageBuffer {
    buffer: BytesMut,
}

impl MessageBuffer {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(4096),
        }
    }

    pub fn try_parse_client_message(&mut self) -> Result<Option<ClientMessage>> {
        if self.buffer.len() < 5 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!("Message too large: {} bytes", len);
        }

        if self.buffer.len() < 4 + len {
            return Ok(None);
        }

        // Check type byte
        if self.buffer[4] != MESSAGE_TYPE_CLIENT {
            return Ok(None);
        }

        self.buffer.advance(4); // Skip length
        self.buffer.advance(1); // Skip type byte
        let data = self.buffer.split_to(len - 1); // -1 for type byte
        let msg = ClientMessage::from_bytes(&data)?;

        Ok(Some(msg))
    }

    pub fn try_parse_server_message(&mut self) -> Result<Option<ServerMessage>> {
        if self.buffer.len() < 5 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!("Message too large: {} bytes", len);
        }

        if self.buffer.len() < 4 + len {
            return Ok(None);
        }

        // Check type byte
        if self.buffer[4] != MESSAGE_TYPE_SERVER {
            return Ok(None);
        }

        self.buffer.advance(4); // Skip length
        self.buffer.advance(1); // Skip type byte
        let data = self.buffer.split_to(len - 1); // -1 for type byte
        let msg = ServerMessage::from_bytes(&data)?;

        Ok(Some(msg))
    }

    pub fn try_parse_local_command(&mut self) -> Result<Option<LocalCommand>> {
        if self.buffer.len() < 5 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!("Message too large: {} bytes", len);
        }

        if self.buffer.len() < 4 + len {
            return Ok(None);
        }

        // Check type byte
        if self.buffer[4] != MESSAGE_TYPE_LOCAL_COMMAND {
            return Ok(None);
        }

        self.buffer.advance(4); // Skip length
        self.buffer.advance(1); // Skip type byte
        let data = self.buffer.split_to(len - 1); // -1 for type byte
        let msg = LocalCommand::from_bytes(&data)?;

        Ok(Some(msg))
    }

    pub fn try_parse_local_response(&mut self) -> Result<Option<LocalResponse>> {
        if self.buffer.len() < 5 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if len > MAX_MESSAGE_SIZE {
            anyhow::bail!("Message too large: {} bytes", len);
        }

        if self.buffer.len() < 4 + len {
            return Ok(None);
        }

        // Check type byte
        if self.buffer[4] != MESSAGE_TYPE_LOCAL_RESPONSE {
            return Ok(None);
        }

        self.buffer.advance(4); // Skip length
        self.buffer.advance(1); // Skip type byte
        let data = self.buffer.split_to(len - 1); // -1 for type byte
        let msg = LocalResponse::from_bytes(&data)?;

        Ok(Some(msg))
    }

    pub fn append(&mut self, data: &[u8]) {
        self.buffer.put_slice(data);
    }

    pub fn remaining(&self) -> usize {
        self.buffer.len()
    }
}

impl Default for MessageBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_message_serialization() {
        let msg = ClientMessage::Register {
            hostname: "test-host".to_string(),
            platform: "linux".to_string(),
        };

        let bytes = msg.to_bytes().unwrap();
        let deserialized = ClientMessage::from_bytes(&bytes).unwrap();

        match deserialized {
            ClientMessage::Register {
                hostname,
                platform,
            } => {
                assert_eq!(hostname, "test-host");
                assert_eq!(platform, "linux");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_server_message_serialization() {
        let msg = ServerMessage::Welcome {
            server_version: "1.0.0".to_string(),
            session_id: "session123".to_string(),
        };

        let bytes = msg.to_bytes().unwrap();
        let deserialized = ServerMessage::from_bytes(&bytes).unwrap();

        match deserialized {
            ServerMessage::Welcome {
                server_version,
                session_id,
            } => {
                assert_eq!(server_version, "1.0.0");
                assert_eq!(session_id, "session123");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_message_framing() {
        let msg = ClientMessage::Heartbeat {
            timestamp: 12345,
            sequence: 1,
        };

        let mut buf = Vec::new();
        msg.write_framed(&mut buf).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let received = ClientMessage::read_framed(&mut cursor).unwrap();

        match received {
            ClientMessage::Heartbeat {
                timestamp,
                sequence,
            } => {
                assert_eq!(timestamp, 12345);
                assert_eq!(sequence, 1);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_message_buffer() {
        let msg = ServerMessage::Ping {
            request_id: "req123".to_string(),
        };

        let mut buf = Vec::new();
        msg.write_framed(&mut buf).unwrap();

        let mut msg_buf = MessageBuffer::new();
        msg_buf.append(&buf[..2]);
        assert!(msg_buf.try_parse_server_message().unwrap().is_none());

        msg_buf.append(&buf[2..]);
        let received = msg_buf.try_parse_server_message().unwrap().unwrap();

        match received {
            ServerMessage::Ping { request_id } => {
                assert_eq!(request_id, "req123");
            }
            _ => panic!("Wrong message type"),
        }
    }
}
