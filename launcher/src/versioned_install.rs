//! Versioned deploys with rollback.
//!
//! [`crate::atomic_install`] makes a single deploy safe: the destination is
//! never a partial file. This module makes a *sequence* of deploys
//! recoverable: you can see what was installed when, and put the previous one
//! back without needing the build machine, the network, or the original
//! artifact.
//!
//! That is the difference between a push tool and a distribution mechanism. A
//! bad binary reaching an always-on machine is not a hypothetical — it is the
//! ordinary case that a fleet has to survive, and "rebuild it and push again"
//! is not a recovery plan when the thing you broke is how you reach the box.
//!
//! # Shape
//!
//! The destination stays an ordinary file. It is NOT turned into a symlink
//! into a versions directory, which is the other common design. Reasons:
//!
//! - Anything that stats, execs, or watches the path keeps working unchanged,
//!   including systemd units, and nothing observes a type change mid-deploy.
//! - Rollback reuses [`atomic_install::install_atomic`] exactly, so it inherits
//!   the same tmp-fsync-rename guarantees rather than needing its own.
//! - A symlink flip is atomic too, but it leaves the live path pointing into a
//!   directory that a naive cleanup (or our own pruning) could remove out from
//!   under a running process.
//!
//! Previous versions live in a sidecar beside the destination:
//!
//! ```text
//!   /opt/bin/myservice                          <- the live file
//!   /opt/bin/.myservice.hrl-versions/
//!       manifest.json                           <- ordered deploy history
//!       <sha256>                                <- content, addressed by hash
//! ```
//!
//! Storage is **content-addressed**, so deploying A, then B, then A again
//! stores two blobs, not three, and a rollback-then-redeploy costs nothing.
//!
//! # What is deliberately loud
//!
//! A stored version is re-verified against its checksum before it is installed
//! during a rollback. If the sidecar has been corrupted, the rollback fails and
//! the live file is left alone. Installing unverified bytes over a working
//! binary because we hoped the disk was fine is exactly the silent fallback we
//! would rather crash than perform.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_install::install_atomic;
use crate::rsync_utils::compute_checksum;

/// How many distinct versions to retain by default, including the live one.
pub const DEFAULT_KEEP: usize = 5;

/// One entry in a destination's deploy history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// SHA-256 of the content; also its filename in the sidecar.
    pub checksum: String,
    /// Unix permission bits this version was installed with.
    pub mode: Option<u32>,
    /// Seconds since the epoch, when this deploy happened.
    pub installed_at: u64,
    pub size: u64,
}

impl Version {
    /// Short form for humans and CLI output.
    pub fn short(&self) -> &str {
        &self.checksum[..self.checksum.len().min(12)]
    }
}

/// What an install actually did. Callers care: an unchanged redeploy should
/// not be reported as a new version, or history fills with noise and rollback
/// walks back to an identical binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// Content differed; the destination was replaced.
    Replaced(Version),
    /// Content was already live; nothing was written.
    Unchanged(Version),
}

impl Installed {
    pub fn version(&self) -> &Version {
        match self {
            Installed::Replaced(v) | Installed::Unchanged(v) => v,
        }
    }
    pub fn was_replaced(&self) -> bool {
        matches!(self, Installed::Replaced(_))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    /// Deploy history, oldest first. The last entry is what is live.
    history: Vec<Version>,
}

/// Sidecar directory for `dest`.
fn store_dir(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".into());
    parent.join(format!(".{name}.hrl-versions"))
}

fn manifest_path(dest: &Path) -> PathBuf {
    store_dir(dest).join("manifest.json")
}

fn blob_path(dest: &Path, checksum: &str) -> PathBuf {
    store_dir(dest).join(checksum)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_manifest(dest: &Path) -> io::Result<Manifest> {
    match std::fs::read(manifest_path(dest)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            // A manifest we cannot parse is not something to paper over with
            // Default: it means we would silently forget every prior version
            // and a later rollback would claim there is nothing to roll back
            // to. Fail, and let a human look.
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("corrupt version manifest at {:?}: {e}", manifest_path(dest)),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(e),
    }
}

async fn write_manifest(dest: &Path, manifest: &Manifest) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| io::Error::other(format!("serialising manifest: {e}")))?;
    // The manifest is written through the same atomic path as everything else.
    // A half-written manifest is a lost history.
    install_atomic(&manifest_path(dest), &bytes, Some(0o644)).await
}

/// Install `contents` at `dest`, recording it in the version history.
///
/// Returns [`Installed::Unchanged`] without writing anything if the content
/// already matches what the history says is live.
///
/// `keep` distinct versions are retained (including the live one); older blobs
/// are pruned. Pass [`DEFAULT_KEEP`] if you have no opinion.
pub async fn install_versioned(
    dest: &Path,
    contents: &[u8],
    mode: Option<u32>,
    keep: usize,
) -> io::Result<Installed> {
    let checksum = compute_checksum(contents);
    let mut manifest = read_manifest(dest)?;

    let version = Version {
        checksum: checksum.clone(),
        mode,
        installed_at: now_secs(),
        size: contents.len() as u64,
    };

    // Idempotence. Note this checks the manifest AND that the live file really
    // matches — the manifest can be right about intent while someone has
    // edited the destination by hand, and in that case we should repair it.
    if let Some(live) = manifest.history.last()
        && live.checksum == checksum
        && live.mode == mode
        && dest_matches(dest, &checksum)
    {
        return Ok(Installed::Unchanged(live.clone()));
    }

    // Store the blob BEFORE touching the live file. If we die here, the live
    // file is untouched and we have merely leaked a blob.
    std::fs::create_dir_all(store_dir(dest))?;
    let blob = blob_path(dest, &checksum);
    if !blob.exists() {
        install_atomic(&blob, contents, Some(0o600)).await?;
    }

    install_atomic(dest, contents, mode).await?;

    manifest.history.push(version.clone());
    prune(dest, &mut manifest, keep)?;
    write_manifest(dest, &manifest).await?;

    Ok(Installed::Replaced(version))
}

/// Does `dest` already carry deploy history?
///
/// Deliberately the *directory*, not the manifest: a deploy that died between
/// creating the sidecar and writing the manifest leaves the directory with a
/// blob and no manifest, and that destination is still one we have started
/// versioning. Treating it as unversioned would resume the silent downgrade
/// this predicate exists to prevent.
pub fn is_versioned(dest: &Path) -> bool {
    store_dir(dest).exists()
}

/// Should a deploy of `dest` with `mode` be versioned?
///
/// The executable bit decides for a destination we have never seen. It must
/// NOT decide for one that already has history: the same path redeployed
/// without `+x` — a mode that changed upstream, a rule that syncs a wrapper
/// script, a file that stopped being a binary — would otherwise route to a
/// plain atomic install, replace the live file, and leave the manifest naming
/// a version that is no longer on disk. `current()` would then report a lie
/// and a later rollback would restore the wrong bytes.
///
/// The asymmetry is the point. Falling *into* versioning costs a few KB of
/// sidecar. Falling *out* of it silently costs the recovery path on a machine
/// we may not be able to reach again — so a destination that has version
/// history keeps it.
pub fn should_version(dest: &Path, mode: u32) -> bool {
    mode & 0o111 != 0 || is_versioned(dest)
}

/// Does the live file currently hold exactly this content?
fn dest_matches(dest: &Path, checksum: &str) -> bool {
    match std::fs::read(dest) {
        Ok(bytes) => compute_checksum(&bytes) == checksum,
        Err(_) => false,
    }
}

/// Deploy history for `dest`, **newest first**.
pub fn history(dest: &Path) -> io::Result<Vec<Version>> {
    let mut h = read_manifest(dest)?.history;
    h.reverse();
    Ok(h)
}

/// The version currently believed to be live, if any.
pub fn current(dest: &Path) -> io::Result<Option<Version>> {
    Ok(read_manifest(dest)?.history.last().cloned())
}

/// Roll `dest` back to the version deployed before the current one.
///
/// The rolled-back-from version is dropped from the history, so repeated
/// rollbacks walk backwards through time rather than oscillating between two
/// versions.
pub async fn rollback(dest: &Path, keep: usize) -> io::Result<Version> {
    let mut manifest = read_manifest(dest)?;

    if manifest.history.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "nothing to roll back to for {:?}: {} version(s) in history",
                dest,
                manifest.history.len()
            ),
        ));
    }

    let target = manifest.history[manifest.history.len() - 2].clone();
    let blob = blob_path(dest, &target.checksum);

    let contents = std::fs::read(&blob).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "version {} is recorded but its content is missing at {:?}: {e}",
                target.short(),
                blob
            ),
        )
    })?;

    // Verify before installing. A corrupted sidecar must not be written over a
    // working binary just because the manifest said so.
    let actual = compute_checksum(&contents);
    if actual != target.checksum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stored version {} is corrupt (content hashes to {}) — refusing to install it; \
                 the live file has NOT been touched",
                target.short(),
                &actual[..12.min(actual.len())]
            ),
        ));
    }

    install_atomic(dest, &contents, target.mode).await?;

    // Drop the version we rolled away from, then re-record the target as live.
    manifest.history.pop();
    if manifest.history.last().map(|v| &v.checksum) != Some(&target.checksum) {
        manifest.history.push(target.clone());
    }
    prune(dest, &mut manifest, keep)?;
    write_manifest(dest, &manifest).await?;

    Ok(target)
}

/// Trim history to the newest `keep` entries and delete blobs no entry refers
/// to. Content-addressed storage means a blob may be referenced more than
/// once, so reachability is what decides, not position.
fn prune(dest: &Path, manifest: &mut Manifest, keep: usize) -> io::Result<()> {
    let keep = keep.max(1);
    if manifest.history.len() > keep {
        let drop = manifest.history.len() - keep;
        manifest.history.drain(..drop);
    }

    let live: std::collections::HashSet<&str> =
        manifest.history.iter().map(|v| v.checksum.as_str()).collect();

    let dir = store_dir(dest);
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "manifest.json" || name.contains("hrl-tmp") {
            continue;
        }
        if !live.contains(name.as_ref()) {
            // Best effort: a blob we fail to remove is wasted space, not a
            // correctness problem, and must not fail the deploy.
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn install(dest: &Path, body: &str, mode: u32) -> Installed {
        install_versioned(dest, body.as_bytes(), Some(mode), DEFAULT_KEEP)
            .await
            .expect("install failed")
    }

    /// Mirrors the routing decision the client daemon makes on an incoming
    /// sync, so the rule is exercised here rather than only inside a method
    /// that needs a live SSH connection to reach.
    async fn deploy(dest: &Path, body: &str, mode: u32) {
        if should_version(dest, mode) {
            install_versioned(dest, body.as_bytes(), Some(mode), DEFAULT_KEEP)
                .await
                .expect("versioned install failed");
        } else {
            install_atomic(dest, body.as_bytes(), Some(mode))
                .await
                .expect("atomic install failed");
        }
    }

    fn read(dest: &Path) -> String {
        String::from_utf8(std::fs::read(dest).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn history_records_each_deploy_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "v1", 0o755).await;
        install(&dest, "v2", 0o755).await;
        install(&dest, "v3", 0o755).await;

        let h = history(&dest).unwrap();
        assert_eq!(h.len(), 3, "expected three versions, got {h:#?}");
        assert_eq!(h[0].checksum, compute_checksum(b"v3"), "newest must be first");
        assert_eq!(h[2].checksum, compute_checksum(b"v1"));
        assert_eq!(current(&dest).unwrap().unwrap().checksum, compute_checksum(b"v3"));
    }

    #[tokio::test]
    async fn rollback_restores_the_previous_content_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "good", 0o755).await;
        install(&dest, "broken", 0o700).await;
        assert_eq!(read(&dest), "broken");

        let restored = rollback(&dest, DEFAULT_KEEP).await.expect("rollback failed");

        assert_eq!(read(&dest), "good");
        assert_eq!(restored.checksum, compute_checksum(b"good"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
                0o755,
                "rollback must restore the mode the version was deployed with"
            );
        }
    }

    #[tokio::test]
    async fn repeated_rollback_walks_backwards_rather_than_oscillating() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "v1", 0o755).await;
        install(&dest, "v2", 0o755).await;
        install(&dest, "v3", 0o755).await;

        rollback(&dest, DEFAULT_KEEP).await.unwrap();
        assert_eq!(read(&dest), "v2");
        rollback(&dest, DEFAULT_KEEP).await.unwrap();
        assert_eq!(read(&dest), "v1", "second rollback must reach v1, not bounce to v3");
    }

    #[tokio::test]
    async fn rollback_with_no_previous_version_fails_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");
        install(&dest, "only", 0o755).await;

        let err = rollback(&dest, DEFAULT_KEEP).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("nothing to roll back to"),
            "unhelpful error: {err}"
        );
        assert_eq!(read(&dest), "only", "a failed rollback must not touch the file");
    }

    /// The property that justifies verifying instead of trusting: a damaged
    /// sidecar must never be written over a working binary.
    #[tokio::test]
    async fn rollback_refuses_a_corrupted_stored_version() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "good", 0o755).await;
        install(&dest, "current", 0o755).await;

        // Corrupt the stored copy of "good" in place.
        let blob = blob_path(&dest, &compute_checksum(b"good"));
        std::fs::write(&blob, b"tampered").unwrap();

        let err = rollback(&dest, DEFAULT_KEEP).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("corrupt"), "unhelpful error: {err}");
        assert_eq!(
            read(&dest),
            "current",
            "the live file must be untouched when a rollback is refused"
        );
    }

    #[tokio::test]
    async fn rollback_reports_a_missing_blob_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "good", 0o755).await;
        install(&dest, "current", 0o755).await;
        std::fs::remove_file(blob_path(&dest, &compute_checksum(b"good"))).unwrap();

        let err = rollback(&dest, DEFAULT_KEEP).await.unwrap_err();
        assert!(
            err.to_string().contains("content is missing"),
            "unhelpful error: {err}"
        );
        assert_eq!(read(&dest), "current");
    }

    #[tokio::test]
    async fn redeploying_identical_content_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "same", 0o755).await;
        let again = install_versioned(&dest, b"same", Some(0o755), DEFAULT_KEEP)
            .await
            .unwrap();

        assert!(!again.was_replaced(), "identical redeploy should be Unchanged");
        assert_eq!(
            history(&dest).unwrap().len(),
            1,
            "an unchanged redeploy must not add history — otherwise rollback \
             walks back to an identical binary"
        );
    }

    /// The manifest can be right about intent while the file has been changed
    /// underneath us. Repair rather than trust.
    #[tokio::test]
    async fn reinstalls_when_the_live_file_was_modified_behind_our_back() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "correct", 0o755).await;
        std::fs::write(&dest, b"someone edited this").unwrap();

        let again = install_versioned(&dest, b"correct", Some(0o755), DEFAULT_KEEP)
            .await
            .unwrap();

        assert!(again.was_replaced(), "should have repaired the tampered file");
        assert_eq!(read(&dest), "correct");
    }

    #[tokio::test]
    async fn pruning_keeps_the_newest_n_and_deletes_unreferenced_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        for i in 1..=6 {
            install_versioned(&dest, format!("v{i}").as_bytes(), Some(0o755), 3)
                .await
                .unwrap();
        }

        let h = history(&dest).unwrap();
        assert_eq!(h.len(), 3, "keep=3 should retain exactly three versions");
        assert_eq!(h[0].checksum, compute_checksum(b"v6"));
        assert_eq!(h[2].checksum, compute_checksum(b"v4"));

        // v1..v3 blobs must be gone; v4..v6 must remain.
        for old in ["v1", "v2", "v3"] {
            assert!(
                !blob_path(&dest, &compute_checksum(old.as_bytes())).exists(),
                "{old} blob should have been pruned"
            );
        }
        for kept in ["v4", "v5", "v6"] {
            assert!(
                blob_path(&dest, &compute_checksum(kept.as_bytes())).exists(),
                "{kept} blob should have been kept"
            );
        }
    }

    /// Content addressing: redeploying an earlier version must not duplicate
    /// its blob, and must not prune it away while it is still referenced.
    #[tokio::test]
    async fn repeated_content_is_stored_once_and_stays_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        install(&dest, "A", 0o755).await;
        install(&dest, "B", 0o755).await;
        install(&dest, "A", 0o755).await;

        let blobs: Vec<_> = std::fs::read_dir(store_dir(&dest))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "manifest.json")
            .collect();
        assert_eq!(blobs.len(), 2, "A stored twice: {blobs:?}");
        assert!(blob_path(&dest, &compute_checksum(b"A")).exists());
        assert_eq!(read(&dest), "A");
    }

    #[tokio::test]
    async fn a_corrupt_manifest_fails_loudly_instead_of_forgetting_history() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");
        install(&dest, "v1", 0o755).await;

        std::fs::write(manifest_path(&dest), b"{not json").unwrap();

        let err = history(&dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("corrupt"), "unhelpful error: {err}");
    }

    /// Rollback must be as atomic as install — it uses the same primitive, and
    /// this asserts that rather than assuming it.
    #[tokio::test]
    async fn rollback_is_never_observable_as_a_partial_file() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        let old = vec![b'A'; 4 * 1024 * 1024];
        let new = vec![b'B'; 4 * 1024 * 1024];
        install_versioned(&dest, &old, Some(0o755), DEFAULT_KEEP).await.unwrap();
        install_versioned(&dest, &new, Some(0o755), DEFAULT_KEEP).await.unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let torn = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let watcher = {
            let (dest, old, new) = (dest.clone(), old.clone(), new.clone());
            let (stop, torn) = (Arc::clone(&stop), Arc::clone(&torn));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(b) = std::fs::read(&dest) {
                        if b != old && b != new {
                            torn.lock().unwrap().push(b.len());
                        }
                    }
                }
            })
        };

        rollback(&dest, DEFAULT_KEEP).await.unwrap();

        stop.store(true, Ordering::Relaxed);
        watcher.join().unwrap();
        let torn = torn.lock().unwrap();
        assert!(torn.is_empty(), "rollback was observed torn at sizes {torn:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), old);
    }

    #[tokio::test]
    async fn a_destination_with_history_keeps_it_when_redeployed_without_the_exec_bit() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        deploy(&dest, "v1", 0o755).await;
        // The same destination, redeployed with a mode that has no +x. Before
        // the routing looked at the sidecar, this fell through to a plain
        // atomic install: the live file changed and the manifest did not.
        deploy(&dest, "v2", 0o644).await;

        assert_eq!(read(&dest), "v2");

        let live = current(&dest)
            .unwrap()
            .expect("history must survive a non-executable redeploy");
        assert_eq!(
            live.checksum,
            compute_checksum(b"v2"),
            "manifest says {} is live but the file on disk is v2",
            live.short()
        );
        assert_eq!(history(&dest).unwrap().len(), 2, "both deploys must be recorded");
    }

    #[tokio::test]
    async fn rollback_after_a_non_executable_redeploy_restores_the_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("binary");

        deploy(&dest, "good", 0o755).await;
        deploy(&dest, "bad", 0o644).await;
        assert_eq!(read(&dest), "bad");

        // The consequence that matters: rollback is the thing that has to work
        // when we cannot reach the machine again. Stale metadata makes it
        // restore the wrong bytes, or claim there is nothing to restore.
        let restored = rollback(&dest, DEFAULT_KEEP).await.expect("rollback failed");

        assert_eq!(read(&dest), "good");
        assert_eq!(restored.checksum, compute_checksum(b"good"));
    }

    #[tokio::test]
    async fn a_plain_file_with_no_history_stays_unversioned() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.toml");

        assert!(!should_version(&dest, 0o644), "a fresh non-executable path must not be versioned");

        deploy(&dest, "key = 1", 0o644).await;
        deploy(&dest, "key = 2", 0o644).await;

        assert_eq!(read(&dest), "key = 2");
        assert!(
            !store_dir(&dest).exists(),
            "config files must not grow a sidecar they never needed"
        );
    }

    #[test]
    fn executables_are_versioned_even_with_no_history() {
        let dir = tempfile::tempdir().unwrap();
        assert!(should_version(&dir.path().join("fresh-binary"), 0o755));
    }
}
