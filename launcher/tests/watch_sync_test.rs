// Integration test for filesystem watching and automatic syncing
//
// This test validates the end-to-end workflow:
// 1. Server starts and accepts connections
// 2. Client daemon connects to server
// 3. Watch is set up on a directory
// 4. File changes trigger automatic sync to connected client
// 5. Client receives and writes the synced file

use anyhow::Result;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

#[tokio::test(flavor = "multi_thread")]
async fn test_watch_sync_integration() -> Result<()> {
    // Set up logging for the test
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    // Create temporary directories for server and client
    let server_watch_dir = TempDir::new()?;
    let client_output_dir = TempDir::new()?;

    log::info!("Server watch dir: {}", server_watch_dir.path().display());
    log::info!("Client output dir: {}", client_output_dir.path().display());

    // Use a random high port to avoid conflicts
    let test_port = 30000 + (std::process::id() % 10000) as u16;
    log::info!("Using test port: {}", test_port);

    // Start the server in a background task
    let server_task = tokio::spawn(async move {
        halfremembered_launcher::ssh_server::SshServer::run(test_port)
            .await
            .expect("Server failed to start");
    });

    // Give server time to start
    sleep(Duration::from_millis(500)).await;

    // Start client daemon in background
    let client_output_path = client_output_dir.path().to_path_buf();
    let hostname = hostname::get()?.to_string_lossy().to_string();

    let client_task = tokio::spawn(async move {
        let mut daemon = halfremembered_launcher::client_daemon::ClientDaemon::new(
            "localhost".to_string(),
            test_port,
            std::env::var("USER").unwrap_or_else(|_| "testuser".to_string()),
            hostname,
        )
        .with_heartbeat_interval(Duration::from_secs(5))
        .with_reconnect_delay(Duration::from_secs(1))
        .with_working_dir(client_output_path);

        daemon.run().await.expect("Client daemon failed");
    });

    // Give client time to connect and register
    sleep(Duration::from_secs(2)).await;

    // Set up watch using LocalCommand
    let watch_path = server_watch_dir.path().to_path_buf();
    let watch_command = halfremembered_protocol::LocalCommand::WatchDirectory {
        path: watch_path.to_string_lossy().to_string(),
        recursive: true,
        include_patterns: vec!["*.txt".to_string()],
        exclude_patterns: vec![],
    };

    // Send watch command to server
    let response = halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
        "localhost",
        test_port,
        &std::env::var("USER").unwrap_or_else(|_| "testuser".to_string()),
        watch_command,
        None,
    )
    .await?;

    match response {
        halfremembered_protocol::LocalResponse::Success { message } => {
            log::info!("Watch set up successfully: {}", message);
        }
        halfremembered_protocol::LocalResponse::Error { message } => {
            anyhow::bail!("Failed to set up watch: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response: {:?}", response);
        }
    }

    // Give the watch time to be fully set up
    sleep(Duration::from_millis(500)).await;

    // Create a test file in the watched directory
    let test_file_path = server_watch_dir.path().join("test.txt");
    let test_content = "Hello from watch sync test!";
    std::fs::write(&test_file_path, test_content)?;
    log::info!("Created test file: {}", test_file_path.display());

    // Wait for sync to happen (debounce + sync time)
    sleep(Duration::from_secs(3)).await;

    // Verify the file was synced to the client
    let synced_file_path = client_output_dir.path().join("test.txt");

    // Check if file exists
    assert!(
        synced_file_path.exists(),
        "Synced file does not exist at: {}",
        synced_file_path.display()
    );

    // Verify content matches
    let synced_content = std::fs::read_to_string(&synced_file_path)?;
    assert_eq!(
        synced_content, test_content,
        "Synced file content does not match"
    );

    log::info!("✓ File successfully synced with correct content!");

    // Test updating the file
    let updated_content = "Updated content from watch sync test!";
    std::fs::write(&test_file_path, updated_content)?;
    log::info!("Updated test file");

    // Wait for sync
    sleep(Duration::from_secs(3)).await;

    // Verify updated content
    let updated_synced_content = std::fs::read_to_string(&synced_file_path)?;
    assert_eq!(
        updated_synced_content, updated_content,
        "Updated file content does not match"
    );

    log::info!("✓ File update successfully synced!");

    // Test that excluded files are not synced
    let excluded_file_path = server_watch_dir.path().join("excluded.rs");
    std::fs::write(&excluded_file_path, "This should not sync")?;
    log::info!("Created excluded file: {}", excluded_file_path.display());

    // Wait a bit
    sleep(Duration::from_secs(2)).await;

    // Verify excluded file was NOT synced
    let excluded_synced_path = client_output_dir.path().join("excluded.rs");
    assert!(
        !excluded_synced_path.exists(),
        "Excluded file should not have been synced"
    );

    log::info!("✓ Excluded file correctly not synced!");

    // Clean up: abort background tasks
    server_task.abort();
    client_task.abort();

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_watch_with_subdirectories() -> Result<()> {
    // Set up logging
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let server_watch_dir = TempDir::new()?;
    let client_output_dir = TempDir::new()?;

    // Create subdirectory
    let subdir = server_watch_dir.path().join("subdir");
    std::fs::create_dir(&subdir)?;

    let test_port = 30100 + (std::process::id() % 10000) as u16;

    // Start server
    let server_task = tokio::spawn(async move {
        halfremembered_launcher::ssh_server::SshServer::run(test_port)
            .await
            .expect("Server failed");
    });

    sleep(Duration::from_millis(500)).await;

    // Start client
    let client_output_path = client_output_dir.path().to_path_buf();
    let hostname = hostname::get()?.to_string_lossy().to_string();

    let client_task = tokio::spawn(async move {
        let mut daemon = halfremembered_launcher::client_daemon::ClientDaemon::new(
            "localhost".to_string(),
            test_port,
            std::env::var("USER").unwrap_or_else(|_| "testuser".to_string()),
            hostname,
        )
        .with_working_dir(client_output_path);

        daemon.run().await.expect("Client failed");
    });

    sleep(Duration::from_secs(2)).await;

    // Set up recursive watch
    let watch_command = halfremembered_protocol::LocalCommand::WatchDirectory {
        path: server_watch_dir.path().to_string_lossy().to_string(),
        recursive: true,
        include_patterns: vec!["*.txt".to_string()],
        exclude_patterns: vec![],
    };

    halfremembered_launcher::ssh_client::SshClientConnection::send_control_command(
        "localhost",
        test_port,
        &std::env::var("USER").unwrap_or_else(|_| "testuser".to_string()),
        watch_command,
        None,
    )
    .await?;

    sleep(Duration::from_millis(500)).await;

    // Create file in subdirectory
    let test_file = subdir.join("nested.txt");
    std::fs::write(&test_file, "nested content")?;
    log::info!("Created nested file: {}", test_file.display());

    sleep(Duration::from_secs(3)).await;

    // Verify nested file was synced
    let synced_nested = client_output_dir.path().join("subdir").join("nested.txt");
    assert!(
        synced_nested.exists(),
        "Nested file should be synced: {}",
        synced_nested.display()
    );

    let content = std::fs::read_to_string(&synced_nested)?;
    assert_eq!(content, "nested content");

    log::info!("✓ Nested file successfully synced!");

    server_task.abort();
    client_task.abort();

    Ok(())
}
