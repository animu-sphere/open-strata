//! Content-addressed digests (§10.3).
//!
//! All runtime and extension artifacts carry a digest identity. We use SHA-256
//! and render it as `sha256:<hex>` so the algorithm travels with the value.

use sha2::{Digest, Sha256};

/// Compute `sha256:<hex>` over the given bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in out {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("sha256:{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // SHA-256 of the empty input.
        assert_eq!(
            sha256_hex(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
