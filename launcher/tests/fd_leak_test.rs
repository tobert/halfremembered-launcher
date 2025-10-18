
use anyhow::{Context, Result};
use halfremembered_launcher::ssh_server::SshServer;
use serial_test::serial;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::time::sleep;

// Helper function to find an available port
async fn find_available_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    Ok(addr.port())
}

struct TestFixture {
    _server_watch_dir: TempDir,
    _client_output_dir: TempDir,
    port: u16,
    user: String,
}

async fn setup_test() -> Result<TestFixture> {
    let port = find_available_port().await?;
    let user = std::env::var("USER").unwrap_or_else(|_| "testuser".to_string());

    // Server setup
    tokio::spawn(async move {
        if let Err(e) = SshServer::run(port).await {
            log::error!("Server failed: {:?}", e);
        }
    });

    // Wait for server to start
    let mut attempts = 0;
    while attempts < 10 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        attempts += 1;
    }
    if attempts == 10 {
        anyhow::bail!("Server did not start in time");
    }

    Ok(TestFixture {
        _server_watch_dir: TempDir::new()?,
        _client_output_dir: TempDir::new()?,
        port,
        user,
    })
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(unix)] // rlimit is Unix-specific
#[serial]
async fn test_fd_leak_on_sync() -> Result<()> {
    // Set a low file descriptor limit for this process
    let (original_soft, original_hard) = rlimit::getrlimit(rlimit::Resource::NOFILE)?;
    let new_soft_limit = 64;

    // Ensure we don't try to set a limit higher than the hard limit
    if new_soft_limit > original_hard {
        log::warn!(
            "Desired soft limit {} is higher than hard limit {}. Skipping test.",
            new_soft_limit,
            original_hard
        );
        return Ok(());
    }

    struct RestoreRlimit {
        soft: u64,
        hard: u64,
    }

    impl Drop for RestoreRlimit {
        fn drop(&mut self) {
            if let Err(e) = rlimit::setrlimit(rlimit::Resource::NOFILE, self.soft, self.hard) {
                log::error!("Failed to restore rlimit: {}", e);
            } else {
                log::info!("Successfully restored rlimit to ({}, {})", self.soft, self.hard);
            }
        }
    }

    let _rlimit_restorer = RestoreRlimit {
        soft: original_soft,
        hard: original_hard,
    };

    rlimit::setrlimit(rlimit::Resource::NOFILE, new_soft_limit, original_hard)
        .context("Failed to set low rlimit for test")?;

    log::info!(
        "Temporarily lowered rlimit to ({}, {}) for test",
        new_soft_limit,
        original_hard
    );

    let fixture = setup_test().await?;

    // Create a directory for the test files
    let sync_dir = TempDir::new()?;
    let num_files_to_create = new_soft_limit + 20; // Exceed the limit

    log::info!(
        "Attempting to sync {} files, which is more than the FD limit of {}",
        num_files_to_create,
        new_soft_limit
    );

    for i in 0..num_files_to_create {
        let file_path = sync_dir.path().join(format!("file_{}.txt", i));
        std::fs::write(&file_path, format!("content {}", i))?;

        let sync_command = halfremembered_protocol::LocalCommand::SyncFile {
            file: file_path.to_string_lossy().to_string(),
            destination: format!("file_{}.txt", i),
        };

        // This will fail if the server process crashes due to FD exhaustion
        let response = halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
            "localhost",
            fixture.port,
            &fixture.user,
            sync_command,
            None,
        )
        .await
        .context(format!("Failed to send sync command for file {}", i))?;

        match response {
            halfremembered_protocol::LocalResponse::Success { .. } => {
                // Expected
            }
            _ => {
                anyhow::bail!("Unexpected response from server: {:?}", response);
            }
        }
        log::info!("Successfully sent sync command for file {}", i);
    }

    log::info!(
        "✓ Successfully sent sync commands for {} files without crashing",
        num_files_to_create
    );

    Ok(())
}
