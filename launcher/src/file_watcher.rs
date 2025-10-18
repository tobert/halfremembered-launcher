// Filesystem watching with automatic file syncing
//
// Uses notify-debouncer-mini for cross-platform filesystem events with debouncing.
// Integrates with the rsync-based file syncing system.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use halfremembered_protocol::WatchInfo;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, Debouncer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

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

/// Filesystem watcher that triggers automatic file syncing
pub struct FileWatcher {
    /// Active watch configurations indexed by canonical path
    watches: Arc<Mutex<HashMap<PathBuf, WatchConfig>>>,
    /// The underlying debounced watcher (type-erased)
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    /// Channel for receiving debounced events (dummy - real receiver is in thread)
    #[allow(dead_code)]
    event_receiver: mpsc::Receiver<Vec<PathBuf>>,
}

impl FileWatcher {
    /// Create a new file watcher with a callback for file changes
    ///
    /// The callback receives (watch_root, relative_path, absolute_path) for each
    /// file that changes and passes filters.
    pub fn new<F>(mut on_change: F) -> Result<Self>
    where
        F: FnMut(PathBuf, PathBuf, PathBuf) + Send + 'static,
    {
        let watches: Arc<Mutex<HashMap<PathBuf, WatchConfig>>> = Arc::new(Mutex::new(HashMap::new()));
        let watches_clone = Arc::clone(&watches);

        // Create channel for debouncer events
        let (debounce_tx, debounce_rx) = mpsc::channel();

        // Create debouncer with 100ms delay
        let debouncer = new_debouncer(
            Duration::from_millis(100),
            debounce_tx,
        )
        .context("Failed to create filesystem watcher")?;

        // Spawn thread to process events
        std::thread::spawn(move || {
            while let Ok(result) = debounce_rx.recv() {
                match result {
                    Ok(events) => {
                        let watches = watches_clone.lock().unwrap();

                        for event in events {
                            let path = event.path;

                            // Only process regular files
                            if !path.is_file() {
                                continue;
                            }

                            // Find which watch(es) this path belongs to
                            for (watch_root, config) in watches.iter() {
                                if config.matches(&path) {
                                    // Compute relative path using config.path (not watch_root key)
                                    // For single files, watch_root is the file itself, but config.path is the parent
                                    let relative = match path.strip_prefix(&config.path) {
                                        Ok(rel) => rel.to_path_buf(),
                                        Err(_) => continue,
                                    };

                                    log::info!(
                                        "File changed: {} (watch: {}, relative: {})",
                                        path.display(),
                                        watch_root.display(),
                                        relative.display()
                                    );

                                    // Call the sync callback
                                    on_change(watch_root.clone(), relative, path.clone());
                                }
                            }
                        }
                    }
                    Err(errors) => {
                        log::error!("Filesystem watch error: {:?}", errors);
                    }
                }
            }
        });

        Ok(Self {
            watches,
            _debouncer: debouncer,
            event_receiver: mpsc::channel().1, // Dummy receiver, real one is in thread
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
            self._debouncer
                .watcher()
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

            self._debouncer
                .watcher()
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

        log::info!("Removing watch: {}", canonical.display());

        self._debouncer
            .watcher()
            .unwatch(&canonical)
            .context(format!("Failed to unwatch directory: {}", canonical.display()))?;

        let mut watches = self.watches.lock().unwrap();
        watches.remove(&canonical);

        Ok(())
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
