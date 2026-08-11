//! Atomic, durable file installation.
//!
//! This exists because the deploy path is how binaries reach always-on
//! machines, and the previous implementation could brick one. It wrote the new
//! contents straight over the destination with `fs::write`, which means:
//!
//! 1. **A partial file is observable at the live path.** Any reader — or the
//!    service manager restarting the unit — can see a truncated executable
//!    while the write is in flight. Interrupt the deploy and it stays that way.
//! 2. **The `ETXTBSY` workaround made it worse.** Writing to a running binary
//!    fails with `ETXTBSY`, so the old code unlinked the destination and
//!    retried. Between the unlink and the write there is **no file at all**.
//!    A crash there leaves the target with nothing to run.
//! 3. **Permissions were applied after the write**, so a fresh deploy was
//!    briefly present but not executable.
//!
//! The standard fix — write to a temp file in the same directory, then
//! `rename` — collapses all three, because `rename(2)` is atomic within a
//! filesystem: a reader sees either the whole old file or the whole new one,
//! never a prefix and never nothing. It also removes the need for the
//! `ETXTBSY` dance entirely: that error is raised when *opening a running
//! binary for writing*, and renaming over one is permitted. Running processes
//! keep their old inode exactly as before.
//!
//! We also `fsync` the file before the rename and the directory after it.
//! That is not pedantry here: the target may well be a machine without a UPS,
//! where writeback is already tuned conservatively for the same reason.
//! Without the file `fsync`, a power cut just after a rename can leave a
//! correctly-named file full of zeros — the rename is journalled, the data is
//! not. Crashing is preferred over corruption; a deploy that fails loudly
//! beats one that silently installs a zero-length binary.

use std::io;
use std::path::{Path, PathBuf};

/// Install `contents` at `dest` atomically and durably.
///
/// On success, `dest` refers to a complete file with the requested `mode`.
/// On failure, `dest` is left exactly as it was, and no temporary file
/// survives.
///
/// `mode` is the Unix permission bits to apply, e.g. `0o755` for an
/// executable. It is applied to the temporary file *before* the rename, so the
/// file is never visible at `dest` with the wrong permissions. Ignored on
/// non-Unix platforms.
pub async fn install_atomic(dest: &Path, contents: &[u8], mode: Option<u32>) -> io::Result<()> {
    let dest = dest.to_path_buf();
    let contents = contents.to_vec();

    // The whole sequence is blocking file I/O including two fsyncs, and there
    // is no async API for fsync-ing a directory. Do it on the blocking pool
    // rather than stalling a runtime worker.
    tokio::task::spawn_blocking(move || install_atomic_blocking(&dest, &contents, mode))
        .await
        .map_err(|e| io::Error::other(format!("install task panicked: {e}")))?
}

fn install_atomic_blocking(dest: &Path, contents: &[u8], mode: Option<u32>) -> io::Result<()> {
    use std::io::Write;

    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    // The temp file MUST live in the destination directory: `rename` is only
    // atomic within a filesystem, and /tmp is very often a different one — a
    // tmpfs, on many systems.
    std::fs::create_dir_all(parent)?;

    let tmp = temp_path_for(dest);

    // Anything that fails from here on must not leave the temp file behind.
    let result = (|| -> io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;

        // Permissions before the rename, so the file is never observable at
        // `dest` with the wrong mode.
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;

        file.write_all(contents)?;
        // Flush the data to disk BEFORE the rename. Without this the rename
        // can be durable while the contents are not.
        file.sync_all()?;
        drop(file);

        std::fs::rename(&tmp, dest)?;

        // And fsync the directory, so the rename itself survives a power cut.
        // Failure here is not fatal to correctness of what is on disk now, but
        // it is worth knowing about, so it propagates.
        let dir = std::fs::File::open(parent)?;
        dir.sync_all()?;

        Ok(())
    })();

    if result.is_err() {
        // Best effort: the original error is what the caller needs to see, so
        // a failure to clean up must not mask it.
        let _ = std::fs::remove_file(&tmp);
    }

    result
}

/// A temp path in the same directory as `dest`, unlikely to collide with a
/// concurrent deploy of the same file.
///
/// Leading dot so it is inconspicuous, and a distinctive infix so an
/// abandoned one (kill -9 between create and cleanup) is obviously ours.
fn temp_path_for(dest: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    parent.join(format!(
        ".{}.hrl-tmp.{}.{}.{}",
        name,
        std::process::id(),
        nanos,
        seq
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Big enough that a non-atomic write takes long enough for the observer
    /// thread to catch it mid-flight. 8 MiB reliably catches `fs::write`.
    const BIG: usize = 8 * 1024 * 1024;

    /// What an observer saw at the destination while an install ran.
    #[derive(Debug, Default)]
    struct Observations {
        old: usize,
        new: usize,
        /// The bug: anything that is neither the complete old nor the complete
        /// new file — a truncated prefix, or the path missing entirely.
        torn: Vec<String>,
    }

    /// A running observer thread polling `dest`; call [`Observer::finish`]
    /// once the install under test has completed.
    struct Observer {
        stop: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
        obs: Arc<std::sync::Mutex<Observations>>,
    }

    impl Observer {
        fn finish(self) -> Observations {
            self.stop.store(true, Ordering::Relaxed);
            self.handle.join().expect("observer thread panicked");
            Arc::try_unwrap(self.obs).unwrap().into_inner().unwrap()
        }
    }

    /// Poll `dest` as fast as possible and record whether it is ever observed
    /// in a state that is neither the complete old file nor the complete new
    /// one.
    ///
    /// This is the part with teeth. Point the test at a non-atomic writer and
    /// it fails; that is the whole reason it exists.
    fn observe(dest: &Path, old: &[u8], new: &[u8]) -> Observer {
        let stop = Arc::new(AtomicBool::new(false));
        let obs = Arc::new(std::sync::Mutex::new(Observations::default()));

        let handle = {
            let dest = dest.to_path_buf();
            let old = old.to_vec();
            let new = new.to_vec();
            let stop = Arc::clone(&stop);
            let obs = Arc::clone(&obs);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match std::fs::read(&dest) {
                        Ok(bytes) => {
                            let mut o = obs.lock().unwrap();
                            if bytes == old {
                                o.old += 1;
                            } else if bytes == new {
                                o.new += 1;
                            } else {
                                o.torn.push(format!("partial file: {} bytes", bytes.len()));
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {
                            obs.lock()
                                .unwrap()
                                .torn
                                .push("destination did not exist".to_string());
                        }
                        // A racing reader can legitimately hit other transient
                        // errors; those are not evidence of tearing.
                        Err(_) => {}
                    }
                }
            })
        };

        Observer { stop, handle, obs }
    }

    #[tokio::test]
    async fn replacing_a_file_is_never_observable_as_partial() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("service-binary");

        let old = vec![b'A'; BIG];
        let new = vec![b'B'; BIG];
        std::fs::write(&dest, &old).unwrap();

        let observer = observe(&dest, &old, &new);
        install_atomic(&dest, &new, Some(0o755))
            .await
            .expect("install failed");
        let obs = observer.finish();

        assert!(
            obs.torn.is_empty(),
            "destination was observed in a torn state {} times: {:?}",
            obs.torn.len(),
            &obs.torn[..obs.torn.len().min(5)]
        );
        // Guard against a vacuous pass: if the observer never actually looked,
        // an empty `torn` proves nothing.
        assert!(
            obs.old + obs.new > 0,
            "observer never read the destination — the test proved nothing"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), new);
    }

    /// The case the old ETXTBSY workaround handled by unlinking, which is
    /// precisely when the destination briefly ceased to exist.
    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_a_running_executable_never_leaves_the_path_empty() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("running-binary");

        // A real running executable, so we exercise the actual ETXTBSY
        // condition rather than a simulation of it.
        let old_script = b"#!/bin/sh\nsleep 30\n".to_vec();
        std::fs::write(&dest, &old_script).unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut child = std::process::Command::new(&dest)
            .spawn()
            .expect("failed to run the test binary");
        // Give the kernel a moment to actually hold the text reference.
        std::thread::sleep(std::time::Duration::from_millis(200));

        let new_script = b"#!/bin/sh\nsleep 31\n".to_vec();
        let observer = observe(&dest, &old_script, &new_script);
        install_atomic(&dest, &new_script, Some(0o755))
            .await
            .expect("install over a running binary failed");
        let obs = observer.finish();

        let _ = child.kill();
        let _ = child.wait();

        assert!(
            obs.torn.is_empty(),
            "destination was observed missing or partial {} times while replacing a RUNNING binary: {:?}",
            obs.torn.len(),
            &obs.torn[..obs.torn.len().min(5)]
        );
        assert_eq!(std::fs::read(&dest).unwrap(), new_script);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn permissions_are_correct_the_instant_the_file_appears() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("fresh-binary");
        let contents = vec![b'X'; BIG];

        let stop = Arc::new(AtomicBool::new(false));
        let bad_modes = Arc::new(std::sync::Mutex::new(Vec::<u32>::new()));

        let watcher = {
            let dest = dest.clone();
            let stop = Arc::clone(&stop);
            let bad_modes = Arc::clone(&bad_modes);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(md) = std::fs::metadata(&dest) {
                        let mode = md.permissions().mode() & 0o777;
                        if mode != 0o755 {
                            bad_modes.lock().unwrap().push(mode);
                        }
                    }
                }
            })
        };

        install_atomic(&dest, &contents, Some(0o755))
            .await
            .expect("install failed");

        stop.store(true, Ordering::Relaxed);
        watcher.join().unwrap();

        let bad = bad_modes.lock().unwrap();
        assert!(
            bad.is_empty(),
            "file was visible at the destination with the wrong mode {} times (e.g. {:o}) \
             — a fresh deploy must never be briefly non-executable",
            bad.len(),
            bad.first().unwrap()
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[tokio::test]
    async fn creates_a_file_that_did_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested").join("new-file");

        install_atomic(&dest, b"hello", Some(0o644))
            .await
            .expect("install failed");

        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn leaves_no_temp_files_behind_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        for i in 0..3 {
            install_atomic(&dest, format!("version {i}").as_bytes(), Some(0o755))
                .await
                .unwrap();
        }

        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("hrl-tmp"))
            .collect();

        assert!(strays.is_empty(), "temp files left behind: {strays:?}");
    }

    #[tokio::test]
    async fn failure_leaves_the_original_intact_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the destination file should be: `rename` onto a
        // non-empty directory fails, which gets us a failing install without
        // having to fake one.
        let dest = dir.path().join("occupied");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("sentinel"), b"do not clobber").unwrap();

        let result = install_atomic(&dest, b"new contents", Some(0o755)).await;
        assert!(result.is_err(), "expected the install to fail");

        assert_eq!(
            std::fs::read(dest.join("sentinel")).unwrap(),
            b"do not clobber",
            "a failed install damaged the destination"
        );

        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("hrl-tmp"))
            .collect();
        assert!(
            strays.is_empty(),
            "a failed install left temp files behind: {strays:?}"
        );
    }
}
