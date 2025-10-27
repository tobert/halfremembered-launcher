// Integration test for filesystem watching and automatic syncing
//
// This test validates the end-to-end workflow:
// 1. Server starts and accepts connections
// 2. Client daemon connects to server
// 3. Watch is set up on a directory
// 4. File changes trigger automatic sync to connected client
// 5. Client receives and writes the synced file

use anyhow::Result;
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio::time::sleep;

// Test fixture that automatically cleans up server and client tasks
struct TestFixture {
    server_task: JoinHandle<()>,
    client_task: JoinHandle<()>,
    server_watch_dir: TempDir,
    client_output_dir: TempDir,
    port: u16,
    user: String,
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        self.server_task.abort();
        self.client_task.abort();
    }
}

// Get an unused TCP port from the OS
fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    Ok(addr.port())
}

// Polling helper: wait for a file to exist with timeout
async fn wait_for_file(path: &Path, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for file: {}", path.display());
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

// Polling helper: wait for file to exist and have expected content
async fn wait_for_file_content(path: &Path, expected: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if content == expected {
                    return Ok(());
                } else {
                    log::debug!(
                        "File exists but content mismatch. Expected: '{}', Got: '{}'",
                        expected,
                        content
                    );
                }
            } else {
                log::debug!("File exists but failed to read");
            }
        } else {
            log::trace!("File does not exist yet: {}", path.display());
        }
        if start.elapsed() > timeout {
            let status = if path.exists() {
                let content = std::fs::read_to_string(path).unwrap_or_else(|_| "<unreadable>".to_string());
                format!("exists with content: '{}'", content)
            } else {
                "does not exist".to_string()
            };
            anyhow::bail!(
                "Timeout waiting for file content at: {} ({})",
                path.display(),
                status
            );
        }
        sleep(Duration::from_millis(100)).await;
    }
}

// Polling helper: wait for file to NOT exist (for testing exclusions)
async fn wait_for_file_absence(path: &Path, duration: Duration) -> Result<()> {
    sleep(duration).await;
    if path.exists() {
        anyhow::bail!("File should not exist: {}", path.display());
    }
    Ok(())
}

// Polling helper: wait for server to be ready to accept connections
async fn wait_for_server_ready(port: u16, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            log::debug!("Server is accepting connections on port {}", port);
            return Ok(());
        }

        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for server to start");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

// Polling helper: wait for at least one client to be connected
async fn wait_for_client_connected(port: u16, user: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let command = halfremembered_protocol::LocalCommand::ListClients;
        if let Ok(response) = halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
            "localhost",
            port,
            user,
            command,
            None,
        )
        .await
        {
            if let halfremembered_protocol::LocalResponse::ClientList { clients } = response {
                if !clients.is_empty() {
                    log::info!("Client connected: {:?}", clients[0].hostname);
                    return Ok(());
                }
            }
        }

        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for client to connect");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

// Coordination helper: wait for sync to complete AND debouncer to settle
// The file watcher uses a 100ms debouncer, so we need to wait for both:
// 1. The file to sync (polling)
// 2. The debouncer window to pass (150ms > 100ms)
async fn wait_for_sync_and_settle(path: &Path, expected: &str, timeout: Duration) -> Result<()> {
    wait_for_file_content(path, expected, timeout).await?;
    // Wait for debouncer to settle before next write
    sleep(Duration::from_millis(150)).await;
    Ok(())
}

// Set up test fixture with server, client, and temporary directories
async fn setup_test() -> Result<TestFixture> {
    // Initialize logging once per test
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    // Create temporary directories
    let server_watch_dir = TempDir::new()?;
    let client_output_dir = TempDir::new()?;

    log::info!("Server watch dir: {}", server_watch_dir.path().display());
    log::info!("Client output dir: {}", client_output_dir.path().display());

    // Use a random high port to avoid conflicts
    let test_port = find_free_port()?;
    log::info!("Using test port: {}", test_port);

    // Start the server in a background task
    let server_task = tokio::spawn(async move {
        halfremembered_launcher::ssh_server::SshServer::run(test_port)
            .await
            .expect("Server failed to start");
    });

    // Wait for server to be ready (poll, not sleep)
    wait_for_server_ready(test_port, Duration::from_secs(2)).await?;

    // Start client daemon in background
    let client_output_path = client_output_dir.path().to_path_buf();
    let hostname = hostname::get()?.to_string_lossy().to_string();
    let user = "testuser".to_string();
    let user_clone = user.clone();

    let client_task = tokio::spawn(async move {
        let mut daemon = halfremembered_launcher::client_daemon::ClientDaemon::new(
            "localhost".to_string(),
            test_port,
            user_clone,
            hostname,
        )
        .with_heartbeat_interval(Duration::from_secs(5))
        .with_reconnect_delay(Duration::from_secs(1))
        .with_working_dir(client_output_path);

        daemon.run().await.expect("Client daemon failed");
    });

    // Wait for client to actually connect and register (poll, don't sleep)
    wait_for_client_connected(test_port, &user, Duration::from_millis(500)).await?;

    Ok(TestFixture {
        server_task,
        client_task,
        server_watch_dir,
        client_output_dir,
        port: test_port,
        user: "testuser".to_string(),
    })
}

// Helper to set up a filesystem watch
async fn setup_watch(
    fixture: &TestFixture,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
) -> Result<()> {
    let watch_command = halfremembered_protocol::LocalCommand::WatchDirectory {
        path: fixture
            .server_watch_dir
            .path()
            .to_string_lossy()
            .to_string(),
        recursive: true,
        include_patterns,
        exclude_patterns,
    };

    let response = halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
        "localhost",
        fixture.port,
        &fixture.user,
        watch_command,
        None,
    )
    .await?;

    match response {
        halfremembered_protocol::LocalResponse::Success { message } => {
            log::info!("Watch set up successfully: {}", message);
            Ok(())
        }
        halfremembered_protocol::LocalResponse::Error { message } => {
            anyhow::bail!("Failed to set up watch: {}", message)
        }
        _ => anyhow::bail!("Unexpected response: {:?}", response),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_watch_sync_integration() -> Result<()> {
    let fixture = setup_test().await?;

    // Set up watch with *.txt files
    setup_watch(&fixture, vec!["*.txt".to_string()], vec![]).await?;

    // Create a test file in the watched directory
    let test_file_path = fixture.server_watch_dir.path().join("test.txt");
    let test_content = "Hello from watch sync test!";
    std::fs::write(&test_file_path, test_content)?;
    log::info!("Created test file: {}", test_file_path.display());

    // Wait for file to sync and debouncer to settle
    let synced_file_path = fixture.client_output_dir.path().join("test.txt");
    wait_for_sync_and_settle(&synced_file_path, test_content, Duration::from_secs(2)).await?;

    log::info!("✓ File successfully synced with correct content!");

    // Test updating the file
    let updated_content = "Updated content from watch sync test!";
    std::fs::write(&test_file_path, updated_content)?;
    log::info!("Updated test file");

    // Wait for updated content to sync and settle
    wait_for_sync_and_settle(&synced_file_path, updated_content, Duration::from_secs(2)).await?;

    log::info!("✓ File update successfully synced!");

    // Test that excluded files are not synced
    let excluded_file_path = fixture.server_watch_dir.path().join("excluded.rs");
    std::fs::write(&excluded_file_path, "This should not sync")?;
    log::info!("Created excluded file: {}", excluded_file_path.display());

    // Wait and verify excluded file was NOT synced
    let excluded_synced_path = fixture.client_output_dir.path().join("excluded.rs");
    wait_for_file_absence(&excluded_synced_path, Duration::from_secs(2)).await?;

    log::info!("✓ Excluded file correctly not synced!");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_watch_with_subdirectories() -> Result<()> {
    let fixture = setup_test().await?;

    // Create subdirectory in watched directory
    let subdir = fixture.server_watch_dir.path().join("subdir");
    std::fs::create_dir(&subdir)?;

    // Set up recursive watch
    setup_watch(&fixture, vec!["*.txt".to_string()], vec![]).await?;

    // Create file in subdirectory
    let test_file = subdir.join("nested.txt");
    let test_content = "nested content";
    std::fs::write(&test_file, test_content)?;
    log::info!("Created nested file: {}", test_file.display());

    // Wait for nested file to sync and settle
    let synced_nested = fixture
        .client_output_dir
        .path()
        .join("subdir")
        .join("nested.txt");
    wait_for_sync_and_settle(&synced_nested, test_content, Duration::from_secs(2)).await?;

    log::info!("✓ Nested file successfully synced!");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_watch_single_file() -> Result<()> {
    let fixture = setup_test().await?;

    // Create a file to watch
    let watched_file = fixture.server_watch_dir.path().join("specific.exe");
    let initial_content = "version 1.0";
    std::fs::write(&watched_file, initial_content)?;
    log::info!("Created file to watch: {}", watched_file.display());

    // Set up watch on the specific file (not the directory)
    let watch_command = halfremembered_protocol::LocalCommand::WatchDirectory {
        path: watched_file.to_string_lossy().to_string(),
        recursive: false,
        include_patterns: vec![],
        exclude_patterns: vec![],
    };

    let response = halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
        "localhost",
        fixture.port,
        &fixture.user,
        watch_command,
        None,
    )
    .await?;

    match response {
        halfremembered_protocol::LocalResponse::Success { message } => {
            log::info!("Watch set up successfully: {}", message);
        }
        halfremembered_protocol::LocalResponse::Error { message } => {
            anyhow::bail!("Failed to set up watch: {}", message)
        }
        _ => anyhow::bail!("Unexpected response: {:?}", response),
    }

    // Modify the watched file
    let updated_content = "version 2.0";
    std::fs::write(&watched_file, updated_content)?;
    log::info!("Updated watched file");

    // Wait for file to sync and debouncer to settle
    let synced_file_path = fixture.client_output_dir.path().join("specific.exe");
    wait_for_sync_and_settle(&synced_file_path, updated_content, Duration::from_secs(2)).await?;

    log::info!("✓ Single file successfully synced with correct content!");

    // Create another file in the same directory - it should NOT sync
    let other_file = fixture.server_watch_dir.path().join("other.txt");
    std::fs::write(&other_file, "should not sync")?;
    log::info!("Created unrelated file in same directory");

    // Verify the other file was NOT synced
    let other_synced = fixture.client_output_dir.path().join("other.txt");
    wait_for_file_absence(&other_synced, Duration::from_secs(2)).await?;

    log::info!("✓ Other files in directory correctly not synced!");

    // Update the watched file again to ensure watch is still active
    let final_content = "version 3.0";
    std::fs::write(&watched_file, final_content)?;
    log::info!("Updated watched file again");

    wait_for_sync_and_settle(&synced_file_path, final_content, Duration::from_secs(2)).await?;

    log::info!("✓ Single file watch remains active after multiple updates!");

    Ok(())
}
