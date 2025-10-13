use anyhow::{Context, Result};
use fast_rsync::{Signature, SignatureOptions};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Default block size for rsync algorithm (4KB)
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

/// Default crypto hash size (full MD4 hash)
pub const DEFAULT_CRYPTO_HASH_SIZE: u32 = 16;

/// Generate signature from file
pub async fn generate_signature(path: &Path, block_size: u32) -> Result<Vec<u8>> {
    let data = tokio::fs::read(path)
        .await
        .context(format!("Failed to read file: {}", path.display()))?;

    let sig = Signature::calculate(
        &data,
        SignatureOptions {
            block_size,
            crypto_hash_size: DEFAULT_CRYPTO_HASH_SIZE,
        },
    );
    Ok(sig.into_serialized())
}

/// Generate delta from source file and signature
pub fn generate_delta(source: &[u8], signature_data: &[u8]) -> Result<Vec<u8>> {
    let sig = if signature_data.is_empty() {
        // Empty signature means no base file - generate signature for empty data
        Signature::calculate(
            &[],
            SignatureOptions {
                block_size: DEFAULT_BLOCK_SIZE,
                crypto_hash_size: DEFAULT_CRYPTO_HASH_SIZE,
            },
        )
    } else {
        Signature::deserialize(signature_data.to_vec())
            .context("Failed to deserialize signature")?
    };

    let index = sig.index();
    let mut delta = Vec::new();

    fast_rsync::diff(&index, source, &mut delta).context("Failed to compute delta")?;

    Ok(delta)
}

/// Apply delta to base file
pub async fn apply_delta(base_path: Option<&Path>, delta_data: &[u8]) -> Result<Vec<u8>> {
    let base = match base_path {
        Some(path) if path.exists() => {
            tokio::fs::read(path)
                .await
                .context(format!("Failed to read base file: {}", path.display()))?
        }
        _ => Vec::new(),
    };

    let mut output = Vec::new();
    fast_rsync::apply(&base, delta_data, &mut output).context("Failed to apply delta")?;

    Ok(output)
}

/// Compute SHA256 checksum of data
pub fn compute_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Choose appropriate block size based on file size
pub fn choose_block_size(file_size: u64) -> u32 {
    if file_size < 1024 * 1024 {
        // < 1 MB: 4 KB blocks
        4096
    } else if file_size < 100 * 1024 * 1024 {
        // 1-100 MB: 4 KB blocks
        4096
    } else if file_size < 500 * 1024 * 1024 {
        // 100-500 MB: 8 KB blocks
        8192
    } else {
        // > 500 MB: 16 KB blocks
        16384
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_generate_signature() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, World!").unwrap();
        temp_file.flush().unwrap();

        let signature = generate_signature(temp_file.path(), DEFAULT_BLOCK_SIZE)
            .await
            .unwrap();

        assert!(!signature.is_empty());
    }

    #[test]
    fn test_generate_delta_identical() {
        let source = b"Hello, World!";
        let sig = Signature::calculate(
            source,
            SignatureOptions {
                block_size: DEFAULT_BLOCK_SIZE,
                crypto_hash_size: DEFAULT_CRYPTO_HASH_SIZE,
            },
        );
        let sig_bytes = sig.serialized();

        let delta = generate_delta(source, sig_bytes).unwrap();

        // Delta for identical file should exist
        assert!(!delta.is_empty());
    }

    #[test]
    fn test_generate_delta_empty_signature() {
        let source = b"Hello, World!";
        let delta = generate_delta(source, &[]).unwrap();

        // Empty signature should result in a proper rsync delta (not raw content)
        assert!(!delta.is_empty());
        // The delta should be different from the source (it has rsync format)
        assert_ne!(delta.as_slice(), source);
    }

    #[tokio::test]
    async fn test_apply_delta_new_file() {
        let source = b"Hello, World!";

        // Generate a delta from empty base
        let empty_sig = Signature::calculate(
            b"",
            SignatureOptions {
                block_size: DEFAULT_BLOCK_SIZE,
                crypto_hash_size: DEFAULT_CRYPTO_HASH_SIZE,
            },
        );
        let delta = generate_delta(source, empty_sig.serialized()).unwrap();

        let result = apply_delta(None, &delta).await.unwrap();

        assert_eq!(result, source);
    }

    #[tokio::test]
    async fn test_apply_delta_with_base() {
        let base = b"Hello, World!";
        let modified = b"Hello, Rust!";

        // Generate signature from base
        let sig = Signature::calculate(
            base,
            SignatureOptions {
                block_size: DEFAULT_BLOCK_SIZE,
                crypto_hash_size: DEFAULT_CRYPTO_HASH_SIZE,
            },
        );
        let index = sig.index();

        // Compute delta
        let mut delta = Vec::new();
        fast_rsync::diff(&index, modified, &mut delta).unwrap();

        // Write base to temp file
        let mut temp_base = NamedTempFile::new().unwrap();
        temp_base.write_all(base).unwrap();
        temp_base.flush().unwrap();

        // Apply delta
        let result = apply_delta(Some(temp_base.path()), &delta)
            .await
            .unwrap();

        assert_eq!(result, modified);
    }

    #[test]
    fn test_compute_checksum() {
        let data = b"Hello, World!";
        let checksum = compute_checksum(data);

        // SHA256 of "Hello, World!" should be consistent
        assert_eq!(checksum.len(), 64); // SHA256 is 32 bytes = 64 hex chars
        assert_eq!(
            checksum,
            "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"
        );
    }

    #[test]
    fn test_compute_checksum_different_data() {
        let data1 = b"Hello, World!";
        let data2 = b"Hello, Rust!";

        let checksum1 = compute_checksum(data1);
        let checksum2 = compute_checksum(data2);

        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn test_choose_block_size() {
        assert_eq!(choose_block_size(500 * 1024), 4096); // 500 KB
        assert_eq!(choose_block_size(5 * 1024 * 1024), 4096); // 5 MB
        assert_eq!(choose_block_size(150 * 1024 * 1024), 8192); // 150 MB
        assert_eq!(choose_block_size(600 * 1024 * 1024), 16384); // 600 MB
    }

    #[tokio::test]
    async fn test_round_trip_signature_delta_apply() {
        // Original file
        let original = b"The quick brown fox jumps over the lazy dog";

        // Write to temp file
        let mut temp_original = NamedTempFile::new().unwrap();
        temp_original.write_all(original).unwrap();
        temp_original.flush().unwrap();

        // Generate signature
        let signature = generate_signature(temp_original.path(), DEFAULT_BLOCK_SIZE)
            .await
            .unwrap();

        // Modified version
        let modified = b"The quick brown fox jumps over the lazy cat";

        // Generate delta
        let delta = generate_delta(modified, &signature).unwrap();

        // Apply delta
        let result = apply_delta(Some(temp_original.path()), &delta)
            .await
            .unwrap();

        assert_eq!(result, modified);
    }
}
