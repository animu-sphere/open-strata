// SPDX-License-Identifier: Apache-2.0
//! Content-addressed digests (§10.3).
//!
//! All runtime and extension artifacts carry a digest identity. We use SHA-256
//! and render it as `sha256:<hex>` so the algorithm travels with the value.

use std::io::{self, Read};

use sha2::{Digest, Sha256};

/// Compute `sha256:<hex>` over the given bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    render(hasher)
}

/// Compute `sha256:<hex>` over a reader without buffering it whole, plus the
/// total bytes read. For large artifacts (a built runtime is GBs) where
/// [`sha256_hex`] over `fs::read` would hold the archive in memory.
pub fn sha256_hex_reader(reader: &mut impl Read) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((render(hasher), total))
}

fn render(hasher: Sha256) -> String {
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

    #[test]
    fn reader_matches_bytes() {
        let data = b"openstrata artifact";
        let mut cursor = std::io::Cursor::new(&data[..]);
        let (digest, size) = sha256_hex_reader(&mut cursor).unwrap();
        assert_eq!(digest, sha256_hex(data));
        assert_eq!(size, data.len() as u64);
    }
}
