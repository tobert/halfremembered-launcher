// Control Messages - Client to Server (0x0001 - 0x000F)
pub const MSG_CLIENT_REGISTER: u16 = 0x0001;
pub const MSG_CLIENT_HEARTBEAT: u16 = 0x0002;
pub const MSG_CLIENT_FILE_RECEIVED: u16 = 0x0003; // Deprecated, use MSG_RSYNC_COMPLETE
pub const MSG_CLIENT_EXEC_COMPLETE: u16 = 0x0004;
pub const MSG_CLIENT_STATUS: u16 = 0x0005;
pub const MSG_CLIENT_ERROR: u16 = 0x0006;

// Control Messages - Server to Client (0x0010 - 0x001F)
pub const MSG_SERVER_WELCOME: u16 = 0x0010;
pub const MSG_SERVER_SYNC_FILE: u16 = 0x0011; // Deprecated, use MSG_RSYNC_START
pub const MSG_SERVER_EXECUTE: u16 = 0x0012;
pub const MSG_SERVER_PING: u16 = 0x0013;
pub const MSG_SERVER_SHUTDOWN: u16 = 0x0014;

// Rsync Messages (0x0100 - 0x01FF)
pub const MSG_RSYNC_START: u16 = 0x0100; // Control channel: initiate sync
pub const MSG_RSYNC_COMPLETE: u16 = 0x0101; // Control channel: sync result
pub const MSG_RSYNC_SIGNATURE: u16 = 0x0102; // Rsync channel: file signature
pub const MSG_RSYNC_DELTA: u16 = 0x0103; // Rsync channel: delta data

// Exec Messages (0x0150 - 0x015F)
pub const MSG_EXEC_HANDSHAKE: u16 = 0x0150; // Exec channel: execute_id handshake
pub const MSG_EXEC_STDOUT: u16 = 0x0151; // Exec channel: stdout data
pub const MSG_EXEC_STDERR: u16 = 0x0152; // Exec channel: stderr data
pub const MSG_EXEC_EXIT: u16 = 0x0153; // Exec channel: exit code

// Stream Messages (0x0200 - 0x02FF)
pub const MSG_STREAM_OPEN: u16 = 0x0200; // Control channel
pub const MSG_STREAM_CLOSE: u16 = 0x0201; // Control channel
pub const MSG_STREAM_ERROR: u16 = 0x0202; // Control channel
pub const MSG_STREAM_DATA: u16 = 0x0210; // Stream channel
pub const MSG_STREAM_EOF: u16 = 0x0211; // Stream channel

// Local Commands (0x0300 - 0x03FF)
pub const MSG_LOCAL_COMMAND: u16 = 0x0300;
pub const MSG_LOCAL_RESPONSE: u16 = 0x0301;

/// Get human-readable name for message type
pub fn message_type_name(msg_type: u16) -> &'static str {
    match msg_type {
        MSG_CLIENT_REGISTER => "ClientRegister",
        MSG_CLIENT_HEARTBEAT => "ClientHeartbeat",
        MSG_CLIENT_FILE_RECEIVED => "ClientFileReceived",
        MSG_CLIENT_EXEC_COMPLETE => "ClientExecComplete",
        MSG_CLIENT_STATUS => "ClientStatus",
        MSG_CLIENT_ERROR => "ClientError",

        MSG_SERVER_WELCOME => "ServerWelcome",
        MSG_SERVER_SYNC_FILE => "ServerSyncFile",
        MSG_SERVER_EXECUTE => "ServerExecute",
        MSG_SERVER_PING => "ServerPing",
        MSG_SERVER_SHUTDOWN => "ServerShutdown",

        MSG_RSYNC_START => "RsyncStart",
        MSG_RSYNC_COMPLETE => "RsyncComplete",
        MSG_RSYNC_SIGNATURE => "RsyncSignature",
        MSG_RSYNC_DELTA => "RsyncDelta",

        MSG_EXEC_HANDSHAKE => "ExecHandshake",
        MSG_EXEC_STDOUT => "ExecStdout",
        MSG_EXEC_STDERR => "ExecStderr",
        MSG_EXEC_EXIT => "ExecExit",

        MSG_STREAM_OPEN => "StreamOpen",
        MSG_STREAM_CLOSE => "StreamClose",
        MSG_STREAM_ERROR => "StreamError",
        MSG_STREAM_DATA => "StreamData",
        MSG_STREAM_EOF => "StreamEof",

        MSG_LOCAL_COMMAND => "LocalCommand",
        MSG_LOCAL_RESPONSE => "LocalResponse",

        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_type_names() {
        assert_eq!(message_type_name(MSG_CLIENT_REGISTER), "ClientRegister");
        assert_eq!(message_type_name(MSG_SERVER_WELCOME), "ServerWelcome");
        assert_eq!(message_type_name(MSG_RSYNC_START), "RsyncStart");
        assert_eq!(message_type_name(MSG_RSYNC_DELTA), "RsyncDelta");
        assert_eq!(message_type_name(MSG_STREAM_DATA), "StreamData");
        assert_eq!(message_type_name(MSG_LOCAL_COMMAND), "LocalCommand");
        assert_eq!(message_type_name(0xFFFF), "Unknown");
    }


    #[test]
    fn test_no_duplicate_constants() {
        // Collect all message type constants
        let types = [MSG_CLIENT_REGISTER,
            MSG_CLIENT_HEARTBEAT,
            MSG_CLIENT_FILE_RECEIVED,
            MSG_CLIENT_EXEC_COMPLETE,
            MSG_CLIENT_STATUS,
            MSG_CLIENT_ERROR,
            MSG_SERVER_WELCOME,
            MSG_SERVER_SYNC_FILE,
            MSG_SERVER_EXECUTE,
            MSG_SERVER_PING,
            MSG_SERVER_SHUTDOWN,
            MSG_RSYNC_START,
            MSG_RSYNC_COMPLETE,
            MSG_RSYNC_SIGNATURE,
            MSG_RSYNC_DELTA,
            MSG_EXEC_HANDSHAKE,
            MSG_EXEC_STDOUT,
            MSG_EXEC_STDERR,
            MSG_EXEC_EXIT,
            MSG_STREAM_OPEN,
            MSG_STREAM_CLOSE,
            MSG_STREAM_ERROR,
            MSG_STREAM_DATA,
            MSG_STREAM_EOF,
            MSG_LOCAL_COMMAND,
            MSG_LOCAL_RESPONSE];

        // Check for duplicates
        for (i, &type1) in types.iter().enumerate() {
            for &type2 in types.iter().skip(i + 1) {
                assert_ne!(
                    type1, type2,
                    "Duplicate message type: 0x{:04x}",
                    type1
                );
            }
        }
    }
}
