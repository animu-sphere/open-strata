// SPDX-License-Identifier: Apache-2.0
//! Measure the real glibc symbol-version floor of a runtime's ELF binaries.
//!
//! A Linux runtime's portability is bounded by the highest `GLIBC_x.y` versioned
//! symbol its binaries reference: a binary that needs `GLIBC_2.43` cannot load on
//! a host whose glibc is older, no matter what ABI label the artifact carries.
//!
//! The variant ABI must therefore be *measured*, not defaulted. `Abi::default_for`
//! stamps a nominal `glibc 2.28`, but a runtime built on a bleeding-edge host
//! (e.g. Ubuntu 26.04 / glibc 2.43) references `GLIBC_2.43` symbols and is
//! unusable on an older runner — while a fabricated `glibc228` label made
//! `--require-target` pass with false confidence (v0.11.0 report, ask #7).
//!
//! We compute the floor by scanning each ELF binary for the `GLIBC_<major>.<minor>`
//! version strings that a versioned-symbol reference records (in `.gnu.version_r`,
//! backed by `.dynstr`) and taking the maximum — the same value
//! `readelf -V <lib> | grep GLIBC_` surfaces, without shelling out to a binutils
//! that may not be installed. Non-ELF files (scripts, resources) and non-Linux
//! runtimes carry no such strings, so the scan is naturally a no-op there.

use std::io::{self, Read};

use camino::Utf8Path;

/// First four bytes of every ELF file: `0x7f 'E' 'L' 'F'`.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// The versioned-symbol prefix we search for.
const GLIBC_PREFIX: &[u8] = b"GLIBC_";

/// Chunk size for streaming a (potentially large) shared library.
const CHUNK: usize = 64 * 1024;

/// Bytes carried between chunks so a `GLIBC_x.y.z` string split across a chunk
/// boundary is still matched. Comfortably longer than any real version string.
const CARRY: usize = 32;

/// A glibc release, ordered by (major, minor). Patch components (`GLIBC_2.43.1`)
/// are rare and do not change the symbol floor, so we key on major.minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
}

impl GlibcVersion {
    /// The version token used in a variant ABI, e.g. `2.43` (which `Abi::token`
    /// renders as `glibc243`).
    pub fn token(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

impl std::fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The maximum `GLIBC_x.y` symbol version referenced across the given files, or
/// `None` if none of them are ELF binaries with a glibc reference (a Windows or
/// macOS runtime, or a runtime that references no versioned glibc symbol).
///
/// Genuine I/O errors reading a file propagate; a non-ELF or unversioned file
/// simply contributes nothing.
pub fn max_glibc_floor<'a, I>(files: I) -> io::Result<Option<GlibcVersion>>
where
    I: IntoIterator<Item = &'a Utf8Path>,
{
    let mut max: Option<GlibcVersion> = None;
    for path in files {
        if let Some(v) = scan_file(path)? {
            max = Some(max.map_or(v, |m| m.max(v)));
        }
    }
    Ok(max)
}

/// Scan a single file for the highest `GLIBC_x.y` reference, streaming so a large
/// shared library is never held whole in memory. Returns `None` for a non-ELF
/// file (checked by magic) or one with no glibc version reference.
fn scan_file(path: &Utf8Path) -> io::Result<Option<GlibcVersion>> {
    let mut file = match std::fs::File::open(path.as_std_path()) {
        Ok(f) => f,
        // A dangling symlink or a file that vanished between staging and scan is
        // not a glibc reference; do not fail the whole measurement over it.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };

    let mut magic = [0u8; 4];
    match read_full(&mut file, &mut magic)? {
        n if n < 4 => return Ok(None), // too short to be ELF
        _ => {}
    }
    if magic != ELF_MAGIC {
        return Ok(None);
    }

    let mut max: Option<GlibcVersion> = None;
    // The window is [carry_from_previous_chunk | freshly_read_chunk]. We scan the
    // whole window, then keep its last CARRY bytes so a boundary-straddling match
    // is seen on the next pass.
    let mut window: Vec<u8> = magic.to_vec();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = read_full(&mut file, &mut buf)?;
        if n > 0 {
            window.extend_from_slice(&buf[..n]);
        }
        scan_window(&window, &mut max);
        if n == 0 {
            break;
        }
        // Retain a tail so a match spanning this chunk and the next is not lost.
        if window.len() > CARRY {
            let tail_start = window.len() - CARRY;
            window.drain(..tail_start);
        }
    }
    Ok(max)
}

/// Find every `GLIBC_<major>.<minor>` occurrence in `bytes` and fold the maximum
/// into `max`.
fn scan_window(bytes: &[u8], max: &mut Option<GlibcVersion>) {
    let mut i = 0;
    while let Some(rel) = find(&bytes[i..], GLIBC_PREFIX) {
        let start = i + rel + GLIBC_PREFIX.len();
        if let Some(v) = parse_version(&bytes[start..]) {
            *max = Some(max.map_or(v, |m| m.max(v)));
        }
        // Advance past this prefix; overlapping `GLIBC_` prefixes cannot occur.
        i = i + rel + GLIBC_PREFIX.len();
    }
}

/// Parse `<major>.<minor>` from the start of `bytes` (trailing `.patch` and any
/// further characters are ignored). Requires at least `major.minor`.
fn parse_version(bytes: &[u8]) -> Option<GlibcVersion> {
    let (major, rest) = take_number(bytes)?;
    let rest = rest.strip_prefix(b".")?;
    let (minor, _) = take_number(rest)?;
    Some(GlibcVersion { major, minor })
}

/// Take a run of ASCII digits from the front of `bytes`, returning the parsed
/// value and the remainder. `None` if the first byte is not a digit.
fn take_number(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let end = bytes
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    // A version component longer than a few digits is not a real glibc version;
    // guard against absurd input rather than overflowing.
    if end > 6 {
        return None;
    }
    let mut value: u32 = 0;
    for &b in &bytes[..end] {
        value = value * 10 + u32::from(b - b'0');
    }
    Some((value, &bytes[end..]))
}

/// First index of `needle` in `haystack` (naive; needle is a fixed 6 bytes).
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read until `buf` is full or EOF, returning bytes read (handles short reads).
fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn v(major: u32, minor: u32) -> GlibcVersion {
        GlibcVersion { major, minor }
    }

    #[test]
    fn parses_major_minor_and_ignores_patch() {
        assert_eq!(parse_version(b"2.43"), Some(v(2, 43)));
        assert_eq!(parse_version(b"2.43.1"), Some(v(2, 43)));
        assert_eq!(parse_version(b"2.28\0next"), Some(v(2, 28)));
        assert_eq!(parse_version(b"2.5"), Some(v(2, 5)));
        assert_eq!(parse_version(b"nope"), None);
        assert_eq!(parse_version(b"2"), None); // needs a minor
    }

    #[test]
    fn version_ordering_is_numeric_not_lexical() {
        // 2.9 < 2.28 < 2.43 numerically (lexical string compare would disagree).
        assert!(v(2, 9) < v(2, 28));
        assert!(v(2, 28) < v(2, 43));
        assert_eq!(
            [v(2, 9), v(2, 43), v(2, 28)].iter().max().copied(),
            Some(v(2, 43))
        );
    }

    #[test]
    fn scans_max_across_a_window() {
        let mut max = None;
        scan_window(b"needs GLIBC_2.17 and GLIBC_2.43 plus GLIBC_2.28", &mut max);
        assert_eq!(max, Some(v(2, 43)));
    }

    fn write_elf(dir: &Utf8Path, name: &str, body: &[u8]) -> Utf8PathBuf {
        let mut bytes = ELF_MAGIC.to_vec();
        bytes.extend_from_slice(body);
        let path = dir.join(name);
        std::fs::write(path.as_std_path(), bytes).unwrap();
        path
    }

    #[test]
    fn floor_is_the_max_over_elf_files_only() {
        let dir = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-glibc-{}", std::process::id())),
        )
        .unwrap();
        std::fs::create_dir_all(dir.as_std_path()).unwrap();

        let low = write_elf(&dir, "liblow.so", b"\x00\x00 GLIBC_2.17 GLIBC_2.28 blah");
        let high = write_elf(&dir, "libhigh.so", b"padding GLIBC_2.43 padding");
        // A non-ELF file that merely contains the string must be ignored.
        let script = dir.join("notes.txt");
        std::fs::write(script.as_std_path(), b"we target GLIBC_2.99 someday").unwrap();

        let files = [low.as_path(), high.as_path(), script.as_path()];
        let floor = max_glibc_floor(files).unwrap();
        assert_eq!(floor, Some(v(2, 43)));

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn no_elf_files_means_no_floor() {
        let dir = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-glibc-none-{}", std::process::id())),
        )
        .unwrap();
        std::fs::create_dir_all(dir.as_std_path()).unwrap();
        let txt = dir.join("readme.txt");
        std::fs::write(txt.as_std_path(), b"GLIBC_2.43 mentioned in prose").unwrap();
        let missing = dir.join("gone.so");

        let files = [txt.as_path(), missing.as_path()];
        assert_eq!(max_glibc_floor(files).unwrap(), None);

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }

    #[test]
    fn matches_across_a_chunk_boundary() {
        // Place a GLIBC string so it straddles the CHUNK boundary: the prefix
        // ends in one read, the version begins in the next.
        let dir = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("ost-glibc-boundary-{}", std::process::id())),
        )
        .unwrap();
        std::fs::create_dir_all(dir.as_std_path()).unwrap();

        let mut body = vec![b'.'; CHUNK - 4]; // magic(4) + this puts us at CHUNK
        body.extend_from_slice(b"GLIBC_2.40");
        let f = write_elf(&dir, "libedge.so", &body);
        assert_eq!(max_glibc_floor([f.as_path()]).unwrap(), Some(v(2, 40)));

        std::fs::remove_dir_all(dir.as_std_path()).ok();
    }
}
