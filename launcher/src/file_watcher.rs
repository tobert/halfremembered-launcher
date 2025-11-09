// Filesystem watching with automatic file syncing
//
// Uses custom debouncer with time-based and checksum-based deduplication.
// Integrates with the rsync-based file syncing system.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use halfremembered_protocol::WatchInfo;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration for a single watch
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Canonical absolute path being watched
    pub path: PathBuf,
    /// Whether to watch recursively
    pub recursive: bool,
    /// Compiled include patterns (empty = include all)
    pub include: GlobSet,
    /// Compiled exclude patterns
    pub exclude: GlobSet,
    /// Original pattern strings for reporting
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

impl WatchConfig {
    /// Create a new watch configuration with pattern compilation
    pub fn new(
        path: PathBuf,
        recursive: bool,
        include_patterns: Vec<String>,
        exclude_patterns: Vec<String>,
    ) -> Result<Self> {
        // Compile include patterns
        let mut include_builder = GlobSetBuilder::new();
        for pattern in &include_patterns {
            let glob = Glob::new(pattern)
                .context(format!("Invalid include pattern: {}", pattern))?;
            include_builder.add(glob);
        }
        let include = include_builder
            .build()
            .context("Failed to compile include patterns")?;

        // Compile exclude patterns
        let mut exclude_builder = GlobSetBuilder::new();
        for pattern in &exclude_patterns {
            let glob = Glob::new(pattern)
                .context(format!("Invalid exclude pattern: {}", pattern))?;
            exclude_builder.add(glob);
        }
        let exclude = exclude_builder
            .build()
            .context("Failed to compile exclude patterns")?;

        Ok(Self {
            path,
            recursive,
            include,
            exclude,
            include_patterns,
            exclude_patterns,
        })
    }

    /// Check if a path matches this watch's filters
    pub fn matches(&self, path: &Path) -> bool {
        // Get relative path from watch root
        let relative = match path.strip_prefix(&self.path) {
            Ok(rel) => rel,
            Err(_) => return false, // Not under this watch root
        };

        let relative_str = relative.to_string_lossy().to_string();

        // If include patterns specified, must match at least one
        if !self.include_patterns.is_empty() && !self.include.is_match(&relative_str) {
            return false;
        }

        // Must not match any exclude pattern
        if self.exclude.is_match(&relative_str) {
            return false;
        }

        true
    }
}

/// State tracking for each watched file
#[derive(Debug, Clone)]
struct FileState {
    /// Last time an event was processed for this file
    last_event_time: Instant,
    /// Last known checksum of the file content
    last_checksum: String,
}

/// Compute SHA-256 checksum of file data (synchronous version for std::thread context)
fn compute_checksum_sync(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Filesystem watcher that triggers automatic file syncing
pub struct FileWatcher {
    /// Active watch configurations indexed by canonical path
    watches: Arc<Mutex<HashMap<PathBuf, WatchConfig>>>,
    /// Per-file state for debouncing and checksum tracking
    file_states: Arc<Mutex<HashMap<PathBuf, FileState>>>,
    /// The underlying notify watcher
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Create a new file watcher with a callback for file changes
    ///
    /// The callback receives (watch_root, relative_path, absolute_path) for each
    /// file that changes and passes filters (time-based debouncing + checksum verification).
    pub fn new<F>(mut on_change: F) -> Result<Self>
    where
        F: FnMut(PathBuf, PathBuf, PathBuf) + Send + 'static,
    {
        let watches: Arc<Mutex<HashMap<PathBuf, WatchConfig>>> = Arc::new(Mutex::new(HashMap::new()));
        let watches_clone = Arc::clone(&watches);

        let file_states: Arc<Mutex<HashMap<PathBuf, FileState>>> = Arc::new(Mutex::new(HashMap::new()));
        let file_states_clone = Arc::clone(&file_states);

        // Create raw notify watcher with custom event handler
        let watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        // Filter 1: Only process data modification and file creation events
                        // Create events are needed because cargo uses hardlinks for final binaries
                        if !matches!(event.kind, EventKind::Modify(ModifyKind::Data(_)) | EventKind::Create(_)) {
                            log::trace!("Ignoring non-data/create event: {:?}", event.kind);
                            return;
                        }

                        for path in event.paths {
                            // Only process regular files
                            if !path.is_file() {
                                continue;
                            }

                            // Filter 2: Time-based debounce (100ms window)
                            let should_process = {
                                let states = file_states_clone.lock().unwrap();
                                if let Some(state) = states.get(&path) {
                                    if state.last_event_time.elapsed() < Duration::from_millis(100) {
                                        log::trace!("⏱️  Debouncing {}", path.display());
                                        return;
                                    }
                                }
                                true
                            };

                            if !should_process {
                                continue;
                            }

                            // Filter 3: Checksum-based deduplication
                            let current_checksum = match std::fs::read(&path) {
                                Ok(data) => compute_checksum_sync(&data),
                                Err(e) => {
                                    log::warn!("Failed to read {} for checksum: {:#}", path.display(), e);
                                    continue;
                                }
                            };

                            let should_callback = {
                                let mut states = file_states_clone.lock().unwrap();
                                if let Some(state) = states.get_mut(&path) {
                                    if state.last_checksum == current_checksum {
                                        log::trace!("⏭️  Skipping {} (checksum unchanged: {})", path.display(), &current_checksum[..8]);
                                        state.last_event_time = Instant::now();
                                        false
                                    } else {
                                        state.last_event_time = Instant::now();
                                        state.last_checksum = current_checksum.clone();
                                        true
                                    }
                                } else {
                                    states.insert(path.clone(), FileState {
                                        last_event_time: Instant::now(),
                                        last_checksum: current_checksum.clone(),
                                    });
                                    true
                                }
                            };

                            if !should_callback {
                                continue;
                            }

                            // Check if file matches any watch pattern before logging/syncing
                            let watches = watches_clone.lock().unwrap();
                            let mut matched = false;
                            for (watch_root, config) in watches.iter() {
                                if config.matches(&path) {
                                    matched = true;

                                    // Compute relative path using config.path (not watch_root key)
                                    // For single files, watch_root is the file itself, but config.path is the parent
                                    let relative = match path.strip_prefix(&config.path) {
                                        Ok(rel) => rel.to_path_buf(),
                                        Err(_) => continue,
                                    };

                                    // Log only files that match patterns
                                    let states = file_states_clone.lock().unwrap();
                                    if let Some(state) = states.get(&path) {
                                        log::info!("📝 File changed: {} (checksum: {} → {})", path.display(), &state.last_checksum[..8], &current_checksum[..8]);
                                    } else {
                                        log::info!("📝 New file: {} (checksum: {})", path.display(), &current_checksum[..8]);
                                    }

                                    // Call the sync callback
                                    on_change(watch_root.clone(), relative, path.clone());
                                    break; // Only process once per file
                                }
                            }

                            if !matched {
                                log::trace!("⏭️  Skipping {} (no matching patterns)", path.display());
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Filesystem watch error: {:?}", e);
                    }
                }
            },
            notify::Config::default(),
        )
        .context("Failed to create filesystem watcher")?;

        Ok(Self {
            watches,
            file_states,
            _watcher: watcher,
        })
    }

    /// Add a file or directory to watch
    pub fn add_watch(
        &mut self,
        path: PathBuf,
        recursive: bool,
        include_patterns: Vec<String>,
        exclude_patterns: Vec<String>,
    ) -> Result<()> {
        // Canonicalize path
        let canonical = path
            .canonicalize()
            .context(format!("Failed to canonicalize path: {}", path.display()))?;

        let is_file = canonical.is_file();
        let is_dir = canonical.is_dir();

        if !is_file && !is_dir {
            anyhow::bail!("Path is neither a file nor directory: {}", canonical.display());
        }

        if is_file {
            log::info!(
                "Adding watch for file: {} (resolved: {})",
                path.display(),
                canonical.display()
            );

            // For single files, watch the parent directory with a filter
            let parent = canonical.parent()
                .context(format!("File has no parent directory: {}", canonical.display()))?
                .to_path_buf();

            let file_name = canonical.file_name()
                .context(format!("Cannot get filename: {}", canonical.display()))?
                .to_string_lossy()
                .to_string();

            // Create watch configuration for the parent directory with file filter
            let config = WatchConfig::new(
                parent.clone(),
                false, // Non-recursive for single file
                vec![file_name.clone()], // Only watch this specific file
                exclude_patterns,
            )?;

            // Watch the parent directory non-recursively
            self._watcher
                .watch(&parent, RecursiveMode::NonRecursive)
                .context(format!("Failed to watch parent directory: {}", parent.display()))?;

            // Store configuration keyed by the actual file path, not parent
            let mut watches = self.watches.lock().unwrap();
            watches.insert(canonical, config);
        } else {
            log::info!(
                "Adding watch for directory: {} (resolved: {}, recursive: {}, include: {:?}, exclude: {:?})",
                path.display(),
                canonical.display(),
                recursive,
                include_patterns,
                exclude_patterns
            );

            // Create watch configuration
            let config = WatchConfig::new(
                canonical.clone(),
                recursive,
                include_patterns,
                exclude_patterns,
            )?;

            // Add to watcher
            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };

            self._watcher
                .watch(&canonical, mode)
                .context(format!("Failed to watch directory: {}", canonical.display()))?;

            // Store configuration
            let mut watches = self.watches.lock().unwrap();
            watches.insert(canonical, config);
        }

        Ok(())
    }

    /// Remove a watch
    pub fn remove_watch(&mut self, path: &Path) -> Result<()> {
        let canonical = path
            .canonicalize()
            .context(format!("Failed to canonicalize path: {}", path.display()))?;

        log::info!("Removing watch for: {}", canonical.display());

        let mut watches = self.watches.lock().unwrap();

        // Find the config for the watch being removed
        if let Some(config_to_remove) = watches.get(&canonical) {
            // This is the actual path that was passed to notify::watch
            let watched_path = if canonical.is_dir() {
                &canonical
            } else {
                // For files, we watched the parent
                &config_to_remove.path
            };

            // Before removing the underlying watch, check if any *other* watches
            // are using the same watched_path. This is crucial for multiple
            // single-file watches in the same directory.
            let is_shared = watches.iter().any(|(watch_key, config)| {
                // Ignore the watch we are about to remove
                if watch_key == &canonical {
                    return false;
                }

                // Determine the underlying watched path for this other watch
                let other_watched_path = if watch_key.is_dir() {
                    watch_key
                } else {
                    &config.path
                };

                other_watched_path == watched_path
            });

            if is_shared {
                log::debug!(
                    "Not unwatching {}. It's shared by other watches.",
                    watched_path.display()
                );
            } else {
                log::debug!("Unwatching {}", watched_path.display());
                self._watcher
                    .unwatch(watched_path)
                    .context(format!("Failed to unwatch path: {}", watched_path.display()))?;
            }

            // Always remove the specific watch config from our map
            watches.remove(&canonical);

            Ok(())
        } else {
            // Don't error if watch doesn't exist, just log it.
            // This can happen in tests or if a remove is duplicated.
            log::warn!("Watch not found for path: {}", canonical.display());
            Ok(())
        }
    }

    /// List all active watches
    pub fn list_watches(&self) -> Vec<WatchInfo> {
        let watches = self.watches.lock().unwrap();
        watches
            .values()
            .map(|config| WatchInfo {
                path: config.path.to_string_lossy().to_string(),
                recursive: config.recursive,
                include_patterns: config.include_patterns.clone(),
                exclude_patterns: config.exclude_patterns.clone(),
            })
            .collect()
    }

    /// Get all files currently matching watch patterns
    ///
    /// Returns (watch_root, relative_path, absolute_path) for each file.
    /// This is used for initial sync when a client connects.
    pub fn get_all_watched_files(&self) -> Vec<(PathBuf, PathBuf, PathBuf)> {
        let watches = self.watches.lock().unwrap();
        let mut files = Vec::new();

        for (watch_root, config) in watches.iter() {
            if watch_root.is_file() {
                // Single file watch - just return the file itself
                let relative = match watch_root.strip_prefix(&config.path) {
                    Ok(rel) => rel.to_path_buf(),
                    Err(_) => {
                        log::warn!("Failed to compute relative path for: {}", watch_root.display());
                        continue;
                    }
                };
                files.push((watch_root.clone(), relative, watch_root.clone()));
            } else if watch_root.is_dir() {
                // Directory watch - walk the tree and find matching files
                let walker = if config.recursive {
                    walkdir::WalkDir::new(watch_root)
                } else {
                    walkdir::WalkDir::new(watch_root).max_depth(1)
                };

                for entry in walker.into_iter().filter_map(|e| e.ok()) {
                    let path = entry.path();

                    // Only process files
                    if !path.is_file() {
                        continue;
                    }

                    // Check if it matches the watch config's filters
                    if config.matches(path) {
                        let relative = match path.strip_prefix(&config.path) {
                            Ok(rel) => rel.to_path_buf(),
                            Err(_) => {
                                log::warn!("Failed to compute relative path for: {}", path.display());
                                continue;
                            }
                        };
                        files.push((watch_root.clone(), relative, path.to_path_buf()));
                    }
                }
            }
        }

        log::debug!("Found {} watched files for initial sync", files.len());
        files
    }

    /// Get all files matching a specific watch path
    pub fn get_files_for_path(&self, path: &Path) -> Vec<(PathBuf, PathBuf, PathBuf)> {
        let watches = self.watches.lock().unwrap();
        let mut files = Vec::new();

        if let Some(config) = watches.get(path) {
            let watch_root = path;
            if watch_root.is_file() {
                // Single file watch
                let relative = match watch_root.strip_prefix(&config.path) {
                    Ok(rel) => rel.to_path_buf(),
                    Err(_) => {
                        log::warn!("Failed to compute relative path for: {}", watch_root.display());
                        return files;
                    }
                };
                files.push((watch_root.to_path_buf(), relative, watch_root.to_path_buf()));
            } else if watch_root.is_dir() {
                // Directory watch
                let walker = if config.recursive {
                    walkdir::WalkDir::new(watch_root)
                } else {
                    walkdir::WalkDir::new(watch_root).max_depth(1)
                };

                for entry in walker.into_iter().filter_map(|e| e.ok()) {
                    let p = entry.path();
                    if p.is_file() && config.matches(p) {
                        let relative = match p.strip_prefix(&config.path) {
                            Ok(rel) => rel.to_path_buf(),
                            Err(_) => {
                                log::warn!("Failed to compute relative path for: {}", p.display());
                                continue;
                            }
                        };
                        files.push((watch_root.to_path_buf(), relative, p.to_path_buf()));
                    }
                }
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_watch_config_matching() {
        let temp = tempdir().unwrap();
        let watch_root = temp.path().to_path_buf();

        let config = WatchConfig::new(
            watch_root.clone(),
            true,
            vec!["*.rs".to_string(), "*.toml".to_string()],
            vec!["target/**".to_string()],
        )
        .unwrap();

        // Should match .rs files
        let rs_file = watch_root.join("src/main.rs");
        assert!(config.matches(&rs_file));

        // Should match .toml files
        let toml_file = watch_root.join("Cargo.toml");
        assert!(config.matches(&toml_file));

        // Should not match excluded target directory
        let target_file = watch_root.join("target/debug/app");
        assert!(!config.matches(&target_file));

        // Should not match non-matching extension
        let txt_file = watch_root.join("README.txt");
        assert!(!config.matches(&txt_file));
    }

    #[test]
    fn test_watch_config_no_include_patterns() {
        let temp = tempdir().unwrap();
        let watch_root = temp.path().to_path_buf();

        // No include patterns = match everything (except excludes)
        let config = WatchConfig::new(
            watch_root.clone(),
            true,
            vec![],
            vec!["*.tmp".to_string()],
        )
        .unwrap();

        assert!(config.matches(&watch_root.join("any/file.rs")));
        assert!(config.matches(&watch_root.join("any/file.txt")));
        assert!(!config.matches(&watch_root.join("temp.tmp")));
    }
}
