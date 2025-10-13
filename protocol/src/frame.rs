use anyhow::{Context, Result};
use bytes::{Buf, BufMut, BytesMut};
use std::io::{Read, Write};

/// Maximum frame size (100 MB)
pub const MAX_FRAME_SIZE: usize = 100 * 1024 * 1024;

/// Frame header size (6 bytes)
pub const FRAME_HEADER_SIZE: usize = 6;

/// Unified frame structure for all messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Message type identifier
    pub message_type: u16,

    /// Message payload
    pub payload: Vec<u8>,
}

impl Frame {
    /// Create new frame with message type and payload
    pub fn new(message_type: u16, payload: Vec<u8>) -> Self {
        Frame {
            message_type,
            payload,
        }
    }

    /// Get total frame size (header + payload)
    pub fn size(&self) -> usize {
        FRAME_HEADER_SIZE + self.payload.len()
    }

    /// Write frame to writer
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Length = type (2) + payload
        let length = (2 + self.payload.len()) as u32;

        if length as usize > MAX_FRAME_SIZE {
            anyhow::bail!(
                "Frame too large: {} bytes (max: {})",
                length,
                MAX_FRAME_SIZE
            );
        }

        // Write header
        writer
            .write_all(&length.to_be_bytes())
            .context("Failed to write frame length")?;
        writer
            .write_all(&self.message_type.to_be_bytes())
            .context("Failed to write message type")?;

        // Write payload
        writer
            .write_all(&self.payload)
            .context("Failed to write frame payload")?;
        writer.flush().context("Failed to flush writer")?;

        log::trace!(
            "Sent frame: type=0x{:04x}, size={} bytes",
            self.message_type,
            length
        );

        Ok(())
    }

    /// Read frame from reader
    pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
        // Read length
        let mut len_bytes = [0u8; 4];
        reader
            .read_exact(&mut len_bytes)
            .context("Failed to read frame length")?;
        let length = u32::from_be_bytes(len_bytes) as usize;

        if length > MAX_FRAME_SIZE {
            anyhow::bail!(
                "Frame too large: {} bytes (max: {})",
                length,
                MAX_FRAME_SIZE
            );
        }

        if length < 2 {
            anyhow::bail!("Frame too small: {} bytes (min: 2)", length);
        }

        // Read message type
        let mut type_bytes = [0u8; 2];
        reader
            .read_exact(&mut type_bytes)
            .context("Failed to read message type")?;
        let message_type = u16::from_be_bytes(type_bytes);

        // Read payload
        let payload_len = length - 2;
        let mut payload = vec![0u8; payload_len];
        reader
            .read_exact(&mut payload)
            .context("Failed to read frame payload")?;

        log::trace!(
            "Received frame: type=0x{:04x}, size={} bytes",
            message_type,
            length
        );

        Ok(Frame {
            message_type,
            payload,
        })
    }
}

/// Async frame buffer for parsing frames from byte stream
pub struct FrameBuffer {
    buffer: BytesMut,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(8192),
        }
    }

    /// Append data to buffer
    pub fn append(&mut self, data: &[u8]) {
        self.buffer.put_slice(data);
    }

    /// Try to parse a complete frame from buffer
    pub fn try_parse(&mut self) -> Result<Option<Frame>> {
        // Need at least header
        if self.buffer.len() < FRAME_HEADER_SIZE {
            return Ok(None);
        }

        // Parse length
        let length = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if length > MAX_FRAME_SIZE {
            anyhow::bail!("Frame too large: {} bytes", length);
        }

        if length < 2 {
            anyhow::bail!("Frame too small: {} bytes", length);
        }

        // Check if we have complete frame
        let total_size = 4 + length; // 4-byte length prefix + frame
        if self.buffer.len() < total_size {
            return Ok(None);
        }

        // Parse message type
        let message_type = u16::from_be_bytes([self.buffer[4], self.buffer[5]]);

        // Extract payload
        self.buffer.advance(6); // Skip header
        let payload = self.buffer.split_to(length - 2).to_vec();

        Ok(Some(Frame {
            message_type,
            payload,
        }))
    }

    /// Get remaining bytes in buffer
    pub fn remaining(&self) -> usize {
        self.buffer.len()
    }
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_new() {
        let frame = Frame::new(0x0100, vec![1, 2, 3, 4, 5]);
        assert_eq!(frame.message_type, 0x0100);
        assert_eq!(frame.payload, vec![1, 2, 3, 4, 5]);
        assert_eq!(frame.size(), 11); // 6 byte header + 5 byte payload
    }

    #[test]
    fn test_frame_write_read_small() {
        let frame = Frame::new(0x0100, vec![1, 2, 3, 4, 5]);

        let mut buf = Vec::new();
        frame.write(&mut buf).unwrap();

        // Check wire format
        assert_eq!(buf.len(), 11); // 4 + 2 + 5
        assert_eq!(&buf[0..4], &[0, 0, 0, 7]); // Length = 7 (2 + 5)
        assert_eq!(&buf[4..6], &[0x01, 0x00]); // Message type
        assert_eq!(&buf[6..11], &[1, 2, 3, 4, 5]); // Payload

        let mut cursor = std::io::Cursor::new(buf);
        let parsed = Frame::read(&mut cursor).unwrap();

        assert_eq!(parsed.message_type, 0x0100);
        assert_eq!(parsed.payload, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_frame_write_read_empty_payload() {
        let frame = Frame::new(0xFFFF, vec![]);

        let mut buf = Vec::new();
        frame.write(&mut buf).unwrap();

        assert_eq!(buf.len(), 6); // 4 + 2 + 0
        assert_eq!(&buf[0..4], &[0, 0, 0, 2]); // Length = 2 (just type)
        assert_eq!(&buf[4..6], &[0xFF, 0xFF]); // Message type

        let mut cursor = std::io::Cursor::new(buf);
        let parsed = Frame::read(&mut cursor).unwrap();

        assert_eq!(parsed.message_type, 0xFFFF);
        assert_eq!(parsed.payload, vec![]);
    }

    #[test]
    fn test_frame_write_read_large_payload() {
        let large_payload = vec![42u8; 10000];
        let frame = Frame::new(0x0200, large_payload.clone());

        let mut buf = Vec::new();
        frame.write(&mut buf).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let parsed = Frame::read(&mut cursor).unwrap();

        assert_eq!(parsed.message_type, 0x0200);
        assert_eq!(parsed.payload.len(), 10000);
        assert_eq!(parsed.payload, large_payload);
    }

    #[test]
    fn test_frame_too_large() {
        let huge_payload = vec![0u8; MAX_FRAME_SIZE + 1];
        let frame = Frame::new(0x0001, huge_payload);

        let mut buf = Vec::new();
        let result = frame.write(&mut buf);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_frame_buffer_single_frame() {
        let frame = Frame::new(0x0001, vec![1, 2, 3]);

        let mut buf = Vec::new();
        frame.write(&mut buf).unwrap();

        let mut frame_buffer = FrameBuffer::new();
        frame_buffer.append(&buf);

        let parsed = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed.message_type, 0x0001);
        assert_eq!(parsed.payload, vec![1, 2, 3]);

        // No more frames
        assert!(frame_buffer.try_parse().unwrap().is_none());
    }

    #[test]
    fn test_frame_buffer_multiple_frames() {
        let frame1 = Frame::new(0x0001, vec![1, 2, 3]);
        let frame2 = Frame::new(0x0002, vec![4, 5, 6]);
        let frame3 = Frame::new(0x0003, vec![7, 8, 9]);

        let mut buf = Vec::new();
        frame1.write(&mut buf).unwrap();
        frame2.write(&mut buf).unwrap();
        frame3.write(&mut buf).unwrap();

        let mut frame_buffer = FrameBuffer::new();
        frame_buffer.append(&buf);

        let parsed1 = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed1.message_type, 0x0001);
        assert_eq!(parsed1.payload, vec![1, 2, 3]);

        let parsed2 = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed2.message_type, 0x0002);
        assert_eq!(parsed2.payload, vec![4, 5, 6]);

        let parsed3 = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed3.message_type, 0x0003);
        assert_eq!(parsed3.payload, vec![7, 8, 9]);

        // No more frames
        assert!(frame_buffer.try_parse().unwrap().is_none());
    }

    #[test]
    fn test_frame_buffer_partial_data() {
        let frame = Frame::new(0x0100, vec![1, 2, 3, 4, 5]);

        let mut buf = Vec::new();
        frame.write(&mut buf).unwrap();

        let mut frame_buffer = FrameBuffer::new();

        // Append only first 2 bytes (partial header)
        frame_buffer.append(&buf[..2]);
        assert!(frame_buffer.try_parse().unwrap().is_none());

        // Append next 3 bytes (still partial)
        frame_buffer.append(&buf[2..5]);
        assert!(frame_buffer.try_parse().unwrap().is_none());

        // Append rest
        frame_buffer.append(&buf[5..]);
        let parsed = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed.message_type, 0x0100);
        assert_eq!(parsed.payload, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_frame_buffer_incremental_append() {
        let frame1 = Frame::new(0x0001, vec![1, 2]);
        let frame2 = Frame::new(0x0002, vec![3, 4]);

        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();
        frame1.write(&mut buf1).unwrap();
        frame2.write(&mut buf2).unwrap();

        let mut frame_buffer = FrameBuffer::new();

        // Append first frame
        frame_buffer.append(&buf1);
        let parsed1 = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed1.message_type, 0x0001);

        // Append second frame
        frame_buffer.append(&buf2);
        let parsed2 = frame_buffer.try_parse().unwrap().unwrap();
        assert_eq!(parsed2.message_type, 0x0002);

        assert!(frame_buffer.try_parse().unwrap().is_none());
    }

    #[test]
    fn test_frame_buffer_remaining() {
        let mut frame_buffer = FrameBuffer::new();
        assert_eq!(frame_buffer.remaining(), 0);

        frame_buffer.append(&[1, 2, 3]);
        assert_eq!(frame_buffer.remaining(), 3);

        frame_buffer.append(&[4, 5, 6]);
        assert_eq!(frame_buffer.remaining(), 6);
    }

    #[test]
    fn test_frame_message_types() {
        // Test various message type values
        let test_types = vec![
            0x0000, 0x0001, 0x00FF, 0x0100, 0x0200, 0x0300, 0xFFFF,
        ];

        for msg_type in test_types {
            let frame = Frame::new(msg_type, vec![1, 2, 3]);
            let mut buf = Vec::new();
            frame.write(&mut buf).unwrap();

            let mut cursor = std::io::Cursor::new(buf);
            let parsed = Frame::read(&mut cursor).unwrap();

            assert_eq!(parsed.message_type, msg_type);
        }
    }

    #[test]
    fn test_frame_clone_eq() {
        let frame1 = Frame::new(0x0100, vec![1, 2, 3]);
        let frame2 = frame1.clone();

        assert_eq!(frame1, frame2);
        assert_eq!(frame1.message_type, frame2.message_type);
        assert_eq!(frame1.payload, frame2.payload);
    }

    #[test]
    fn test_frame_buffer_too_large() {
        let mut frame_buffer = FrameBuffer::new();

        // Craft a frame that claims to be too large
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME_SIZE as u32 + 1000).to_be_bytes()); // Length
        buf.extend_from_slice(&[0x01, 0x00]); // Type
        buf.extend_from_slice(&[0u8; 10]); // Some payload

        frame_buffer.append(&buf);

        let result = frame_buffer.try_parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_frame_buffer_too_small() {
        let mut frame_buffer = FrameBuffer::new();

        // Craft a frame with invalid length (less than 2)
        // Need at least 6 bytes for header (4 length + 2 type) for validation
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_be_bytes()); // Length = 1 (invalid, must be >= 2)
        buf.extend_from_slice(&[0x01, 0x00]); // Type bytes

        frame_buffer.append(&buf);

        let result = frame_buffer.try_parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }
}
