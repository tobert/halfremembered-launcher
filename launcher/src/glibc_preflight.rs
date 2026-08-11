//! Refuse to activate a binary the target's glibc cannot run.
//!
//! # Why this is a *when*, not an *if*
//!
//! glibc symbol versioning is one-directional. A binary built against a newer
//! glibc fails on an older one with `version 'GLIBC_2.XX' not found`; the
//! reverse is fine. So the exposure is not "two machines might drift apart" —
//! it is specifically **the build box pulling ahead of the target**.
//!
//! That is the default trajectory, not the unlucky case: the build box is
//! somebody's daily driver getting updated constantly, and the target is an
//! always-on appliance nobody logs into. On a rolling distribution the gap
//! opens by itself.
//!
//! The failure lands at service start, on a headless machine, and looks
//! exactly like "the new binary is broken" — indistinguishable at 2am from a
//! bad build. Checking costs microseconds.
//!
//! # Where it has to run
//!
//! **On the target, after the bytes land, before activation.** Checking on the
//! build box proves nothing about the target — that is the entire point. The
//! atomic install path gives exactly this seam: content is verified and staged,
//! and only then does the destination change.
//!
//! # What it does on failure
//!
//! Aborts and reports. It deliberately does **not** auto-roll-back: a silent
//! revert to an older binary hides the real problem and leaves someone
//! wondering why their deploy "worked" but nothing changed. Rollback is right
//! here, but as a decision a human or a caller makes with the diagnosis in
//! hand — which is why the required and available versions are both in the
//! error, and why the requirement is recorded in the deploy metadata.

use std::io;

/// A glibc version as (major, minor). Patch levels do not appear in symbol
/// version tags, so two components is the whole story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion(pub u32, pub u32);

impl std::fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.0, self.1)
    }
}

impl GlibcVersion {
    fn parse(s: &str) -> Option<Self> {
        let mut it = s.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        Some(GlibcVersion(major, minor))
    }
}

/// Highest `GLIBC_x.y` symbol version an ELF image requires, or `None` if it
/// references none (a static binary, or not an ELF at all).
///
/// This reads the version tags out of the image's string data rather than
/// parsing ELF section headers. The tags are literal NUL-terminated strings in
/// `.dynstr`, so scanning finds them without a parser to get wrong or an
/// external tool to depend on — and being a pure function of the bytes, it is
/// exhaustively testable without building fixture binaries.
pub fn required_glibc(image: &[u8]) -> Option<GlibcVersion> {
    const TAG: &[u8] = b"GLIBC_";
    let mut best: Option<GlibcVersion> = None;

    for start in memfind_all(image, TAG) {
        let rest = &image[start + TAG.len()..];
        // Take the version characters that follow; stop at anything else.
        let end = rest
            .iter()
            .position(|c| !(c.is_ascii_digit() || *c == b'.'))
            .unwrap_or(rest.len());
        let Ok(text) = std::str::from_utf8(&rest[..end]) else {
            continue;
        };
        if let Some(v) = GlibcVersion::parse(text) {
            best = Some(best.map_or(v, |b| b.max(v)));
        }
    }
    best
}

fn memfind_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    (0..=haystack.len() - needle.len())
        .filter(|&i| &haystack[i..i + needle.len()] == needle)
        .collect()
}

/// The glibc version this machine provides.
///
/// Asks the dynamic loader, which ships with glibc itself and is therefore
/// always present wherever the question is meaningful — unlike `objdump` or
/// other binutils, which are build-box tools and cannot be assumed on an
/// appliance.
pub fn local_glibc() -> io::Result<GlibcVersion> {
    let out = std::process::Command::new("ldd").arg("--version").output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or_default();
    // e.g. "ldd (GNU libc) 2.44"
    first
        .split_whitespace()
        .last()
        .and_then(GlibcVersion::parse)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("could not read a glibc version from {first:?}"),
            )
        })
}

/// Check that this machine can run `image`.
///
/// `Ok(None)` means the image requires no glibc symbol versions at all.
/// `Ok(Some(v))` means it requires `v` and this machine satisfies it.
pub fn check(image: &[u8]) -> io::Result<Option<GlibcVersion>> {
    let Some(required) = required_glibc(image) else {
        return Ok(None);
    };
    let available = local_glibc()?;

    if required > available {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "binary requires glibc {required} but this machine has {available} — \
                 refusing to activate it. It was built on a newer system; glibc symbol \
                 versioning is one-directional, so this would fail at start with \
                 \"version `GLIBC_{required}' not found\". Rebuild it against \
                 glibc {available} or older, or update this machine. \
                 The currently installed version has NOT been replaced."
            ),
        ));
    }
    Ok(Some(required))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_highest_required_version_not_the_first_or_last() {
        // Ordering here is deliberate: highest is neither first nor last, and
        // 2.9 vs 2.34 catches anyone comparing these as strings.
        let image = b"\x7fELF junk GLIBC_2.34\0 GLIBC_2.9\0 GLIBC_2.39\0 GLIBC_2.2.5\0 tail";
        assert_eq!(required_glibc(image), Some(GlibcVersion(2, 39)));
    }

    #[test]
    fn compares_numerically_rather_than_lexically() {
        assert!(GlibcVersion(2, 9) < GlibcVersion(2, 34));
        assert!(GlibcVersion(2, 39) > GlibcVersion(2, 4));
        let image = b"GLIBC_2.9\0GLIBC_2.10\0";
        assert_eq!(
            required_glibc(image),
            Some(GlibcVersion(2, 10)),
            "2.10 > 2.9 numerically, though '2.10' < '2.9' as text"
        );
    }

    #[test]
    fn no_glibc_references_is_not_an_error() {
        assert_eq!(required_glibc(b"a statically linked thing"), None);
        assert_eq!(required_glibc(b""), None);
        assert_eq!(check(b"no glibc here").unwrap(), None);
    }

    #[test]
    fn ignores_malformed_tags_instead_of_panicking() {
        let image = b"GLIBC_ GLIBC_x.y GLIBC_2 GLIBC_..\0 GLIBC_2.31\0";
        assert_eq!(required_glibc(image), Some(GlibcVersion(2, 31)));
    }

    #[test]
    fn a_truncated_tag_at_end_of_buffer_does_not_panic() {
        assert_eq!(required_glibc(b"trailing GLIBC_"), None);
        assert_eq!(required_glibc(b"trailing GLIBC_2"), None);
        assert_eq!(required_glibc(b"trailing GLIBC_2."), None);
    }

    #[test]
    fn this_machine_reports_a_plausible_glibc() {
        let v = local_glibc().expect("could not read local glibc");
        assert_eq!(v.0, 2, "glibc major should be 2, got {v}");
        assert!(v.1 >= 17, "suspiciously old glibc minor: {v}");
    }

    /// The real binary this crate builds must pass on the machine that built
    /// it. If this ever fails, the check itself is wrong.
    #[test]
    fn a_real_local_binary_passes_the_check() {
        let bytes = std::fs::read("/bin/ls").expect("no /bin/ls to test with");
        let result = check(&bytes).expect("/bin/ls failed its own machine's preflight");
        assert!(
            result.is_some(),
            "/bin/ls should reference glibc symbol versions"
        );
    }

    #[test]
    fn rejects_a_binary_needing_a_newer_glibc_than_we_have() {
        let local = local_glibc().unwrap();
        let impossible = format!("GLIBC_{}.{}\0", local.0, local.1 + 50);
        let err = check(impossible.as_bytes()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let msg = err.to_string();
        assert!(msg.contains("refusing to activate"), "unhelpful: {msg}");
        assert!(
            msg.contains("has NOT been replaced"),
            "the error must say the live binary is untouched: {msg}"
        );
        assert!(
            msg.contains(&local.to_string()),
            "the error must name what this machine actually has: {msg}"
        );
    }

    #[test]
    fn accepts_a_binary_needing_exactly_what_we_have() {
        let local = local_glibc().unwrap();
        let exact = format!("GLIBC_{local}\0");
        assert_eq!(check(exact.as_bytes()).unwrap(), Some(local));
    }
}

#[cfg(test)]
mod cross_check {
    /// Cross-check our string-scanning extractor against `objdump -T`, an
    /// independent implementation that actually parses ELF. Agreement on real
    /// binaries is much stronger evidence than our own fixtures.
    #[test]
    fn agrees_with_objdump_on_real_binaries() {
        if std::process::Command::new("objdump").arg("--version").output().is_err() {
            eprintln!("objdump unavailable; skipping cross-check");
            return;
        }
        let mut checked = 0;
        for path in ["/bin/ls", "/bin/bash", "/usr/bin/ssh", "/usr/bin/systemctl"] {
            let Ok(bytes) = std::fs::read(path) else { continue };
            let ours = super::required_glibc(&bytes);

            let out = std::process::Command::new("objdump").args(["-T", path]).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            // objdump prints the version in parentheses -- "(GLIBC_2.38)" --
            // so the parens must come off before the prefix will match. The
            // first version of this cross-check forgot that, found nothing,
            // and reported a disagreement that was entirely its own fault.
            // A failing test is not automatically evidence about the code
            // under test.
            let theirs = text
                .split_whitespace()
                .map(|t| t.trim_matches(|c: char| c == '(' || c == ')'))
                .filter_map(|t| t.strip_prefix("GLIBC_"))
                .filter_map(super::GlibcVersion::parse)
                .max();

            assert_eq!(ours, theirs, "disagreed with objdump on {path}");
            checked += 1;
        }
        assert!(checked > 0, "cross-check examined no binaries — it proved nothing");
    }
}
