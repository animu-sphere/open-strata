// SPDX-License-Identifier: Apache-2.0
//! Measure the real macOS deployment floor of a runtime's Mach-O binaries.
//!
//! Linux measures its glibc symbol floor into the target string and Windows
//! carries `msvc143`; macOS carried `"abi": "native"`, which asserts nothing. A
//! 26.08 build pinned `CMAKE_OSX_DEPLOYMENT_TARGET=14.5` deliberately, so a
//! 15.2-SDK build would not produce a runtime unloadable on the host that built
//! it — and a consumer could not tell that from the artifact, while
//! `--require-target macos-arm64-py313` passed regardless (usd-vrm-plugins
//! report 30 §1).
//!
//! The floor is *measured*, not declared: every Mach-O binary records the
//! minimum OS it was built for in `LC_BUILD_VERSION` (or the older
//! `LC_VERSION_MIN_MACOSX`), and the runtime as a whole is bounded by the
//! highest of them. Reading the load commands directly means the measurement
//! works from any host — a Linux CI runner inspecting a macOS artifact gets the
//! same answer `otool -l` would give on a Mac.

use std::io::{self, Read, Seek, SeekFrom};

use camino::Utf8Path;

/// 64-bit Mach-O, little-endian (`MH_MAGIC_64`). Every macOS target OpenStrata
/// supports is 64-bit and little-endian; a 32-bit or byte-swapped image is not
/// a runtime binary we produce and contributes nothing.
const MACHO_MAGIC_64: u32 = 0xfeed_facf;

/// Universal ("fat") binary magic, which is big-endian by definition.
const FAT_MAGIC: u32 = 0xcafe_babe;

/// Universal binary magic whose arch table carries 64-bit offsets.
const FAT_MAGIC_64: u32 = 0xcafe_babf;

/// Upper bound on the slices of one universal binary we will follow. Apple ships
/// two; this bounds a corrupt `nfat_arch` rather than trusting a count out of an
/// untrusted file.
const MAX_FAT_SLICES: usize = 64;

/// `LC_VERSION_MIN_MACOSX`: version + sdk, both packed `X.Y.Z`.
const LC_VERSION_MIN_MACOSX: u32 = 0x24;

/// `LC_BUILD_VERSION`: platform, minos, sdk, ntools.
const LC_BUILD_VERSION: u32 = 0x32;

/// `PLATFORM_MACOS` in an `LC_BUILD_VERSION`.
const PLATFORM_MACOS: u32 = 1;

/// Cap on the load-command region we will read from one image. Real Mach-O
/// headers are a few KiB; this bounds a corrupt `sizeofcmds` rather than
/// trusting a length out of an untrusted file.
const MAX_LOAD_COMMANDS: usize = 1 << 20;

/// A macOS version as a deployment floor, ordered by (major, minor).
///
/// The patch component of a deployment target is not part of the label — Apple's
/// own `-mmacosx-version-min` is conventionally `major.minor` — so it is parsed
/// and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MacosVersion {
    pub major: u32,
    pub minor: u32,
}

impl MacosVersion {
    /// Decode Apple's packed `xxxx.yy.zz` version word.
    fn from_packed(packed: u32) -> Option<MacosVersion> {
        let major = packed >> 16;
        let minor = (packed >> 8) & 0xff;
        (major > 0).then_some(MacosVersion { major, minor })
    }

    /// The version token used in a variant ABI, e.g. `14.5` (which `Abi::token`
    /// renders as `macos145`).
    pub fn token(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

impl std::fmt::Display for MacosVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// What a runtime's Mach-O binaries say about the hosts that can load them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacosFloor {
    /// Highest minimum-OS across the scanned binaries: the oldest macOS that can
    /// load *all* of them.
    pub deployment_target: Option<MacosVersion>,
    /// Highest SDK any of them was built against. Recorded because a runtime
    /// built against a newer SDK than its deployment target is the normal,
    /// intended case, and the pair is what a consumer reproduces.
    pub sdk: Option<MacosVersion>,
}

impl MacosFloor {
    pub fn is_empty(&self) -> bool {
        self.deployment_target.is_none() && self.sdk.is_none()
    }

    fn absorb(&mut self, other: MacosFloor) {
        self.deployment_target = max_option(self.deployment_target, other.deployment_target);
        self.sdk = max_option(self.sdk, other.sdk);
    }
}

fn max_option(a: Option<MacosVersion>, b: Option<MacosVersion>) -> Option<MacosVersion> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (some, None) | (None, some) => some,
    }
}

/// The macOS deployment floor and SDK across the given files, or an empty floor
/// when none of them is a macOS Mach-O binary (a Linux or Windows runtime).
///
/// Genuine I/O errors propagate; a non-Mach-O file simply contributes nothing.
pub fn max_macos_floor<'a, I>(files: I) -> io::Result<MacosFloor>
where
    I: IntoIterator<Item = &'a Utf8Path>,
{
    let mut floor = MacosFloor::default();
    for path in files {
        floor.absorb(scan_file(path)?);
    }
    Ok(floor)
}

fn scan_file(path: &Utf8Path) -> io::Result<MacosFloor> {
    // `stage_files` deliberately keeps in-tree symlinks, and a macOS framework
    // bundle links whole directories (`Versions/Current -> A`, `Headers ->
    // Versions/Current/Headers`). Opening a directory succeeds on Unix and only
    // fails at the first read, with EISDIR, so resolve the type up front rather
    // than letting an OpenSubdiv.framework abort the whole measurement. A
    // dangling symlink or a file that vanished between staging and scan is
    // likewise not a Mach-O binary.
    match std::fs::metadata(path.as_std_path()) {
        Ok(metadata) if !metadata.is_file() => return Ok(MacosFloor::default()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MacosFloor::default()),
        Err(error) => return Err(error),
    }
    let mut file = std::fs::File::open(path.as_std_path())?;
    // The header plus its load commands is all we need, and it is at the front
    // of the file — a multi-hundred-megabyte library is never read whole.
    let mut head = Vec::new();
    file.by_ref()
        .take(MAX_LOAD_COMMANDS as u64)
        .read_to_end(&mut head)?;

    // A universal binary's later slices start past that window — a real arm64 +
    // x86_64 dylib puts its second slice megabytes in — so each one is read at
    // its own offset. Scanning only what the head happened to contain would take
    // the maximum over a subset and silently under-report the floor, which is
    // the one direction this measurement must never fail in.
    let Some(offsets) = fat_slice_offsets(&head) else {
        return Ok(scan_image(&head));
    };
    let mut floor = MacosFloor::default();
    for offset in offsets {
        let mut slice = Vec::new();
        if file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        file.by_ref()
            .take(MAX_LOAD_COMMANDS as u64)
            .read_to_end(&mut slice)?;
        floor.absorb(scan_thin(&slice));
    }
    Ok(floor)
}

/// The byte offset of every slice of a universal binary, or `None` when `bytes`
/// does not begin with a fat header.
///
/// Both table shapes are read: `FAT_MAGIC` carries 32-bit offsets in 20-byte
/// entries, `FAT_MAGIC_64` carries 64-bit offsets in 32-byte entries. A
/// truncated table simply ends the walk.
fn fat_slice_offsets(bytes: &[u8]) -> Option<Vec<u64>> {
    let (entry_size, wide) = match read_u32_be(bytes, 0)? {
        FAT_MAGIC => (20usize, false),
        FAT_MAGIC_64 => (32usize, true),
        _ => return None,
    };
    let count = read_u32_be(bytes, 4)? as usize;
    let mut offsets = Vec::new();
    for index in 0..count.min(MAX_FAT_SLICES) {
        let entry = 8 + index * entry_size;
        let offset = if wide {
            read_u64_be(bytes, entry + 8)
        } else {
            read_u32_be(bytes, entry + 8).map(u64::from)
        };
        match offset {
            Some(offset) => offsets.push(offset),
            None => break,
        }
    }
    Some(offsets)
}

/// Scan an in-memory Mach-O (or universal) image for its deployment floor.
///
/// This is the whole answer only when the buffer holds the whole image — the
/// non-universal case, and tests. [`scan_file`] reads each universal slice at
/// its own offset instead, since a real one does not fit in the header window.
fn scan_image(bytes: &[u8]) -> MacosFloor {
    let Some(offsets) = fat_slice_offsets(bytes) else {
        return scan_thin(bytes);
    };
    let mut floor = MacosFloor::default();
    for offset in offsets {
        let Ok(offset) = usize::try_from(offset) else {
            continue;
        };
        if offset < bytes.len() {
            floor.absorb(scan_thin(&bytes[offset..]));
        }
    }
    floor
}

/// Scan one non-universal Mach-O image.
fn scan_thin(bytes: &[u8]) -> MacosFloor {
    let mut floor = MacosFloor::default();
    if read_u32_le(bytes, 0) != Some(MACHO_MAGIC_64) {
        return floor;
    }
    let Some(ncmds) = read_u32_le(bytes, 16) else {
        return floor;
    };
    // The 64-bit header is 32 bytes: magic, cputype, cpusubtype, filetype,
    // ncmds, sizeofcmds, flags, reserved.
    let mut offset = 32usize;
    for _ in 0..ncmds {
        let (Some(cmd), Some(size)) = (read_u32_le(bytes, offset), read_u32_le(bytes, offset + 4))
        else {
            break;
        };
        // A zero or unaligned command size would loop forever on a corrupt file.
        let size = size as usize;
        if size < 8 || offset + size > bytes.len() {
            break;
        }
        match cmd {
            LC_BUILD_VERSION => {
                // platform, minos, sdk, ntools
                if read_u32_le(bytes, offset + 8) == Some(PLATFORM_MACOS) {
                    floor.absorb(MacosFloor {
                        deployment_target: read_u32_le(bytes, offset + 12)
                            .and_then(MacosVersion::from_packed),
                        sdk: read_u32_le(bytes, offset + 16).and_then(MacosVersion::from_packed),
                    });
                }
            }
            LC_VERSION_MIN_MACOSX => {
                floor.absorb(MacosFloor {
                    deployment_target: read_u32_le(bytes, offset + 8)
                        .and_then(MacosVersion::from_packed),
                    sdk: read_u32_le(bytes, offset + 12).and_then(MacosVersion::from_packed),
                });
            }
            _ => {}
        }
        offset += size;
    }
    floor
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes(slice.try_into().ok()?))
}

fn read_u64_be(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset + 8)?;
    Some(u64::from_be_bytes(slice.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn packed(major: u32, minor: u32, patch: u32) -> u32 {
        (major << 16) | (minor << 8) | patch
    }

    /// A 64-bit Mach-O header carrying one `LC_BUILD_VERSION`.
    fn build_version_image(minos: u32, sdk: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MACHO_MAGIC_64.to_le_bytes());
        out.extend_from_slice(&0x0100_000cu32.to_le_bytes()); // cputype arm64
        out.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        out.extend_from_slice(&6u32.to_le_bytes()); // filetype MH_DYLIB
        out.extend_from_slice(&1u32.to_le_bytes()); // ncmds
        out.extend_from_slice(&24u32.to_le_bytes()); // sizeofcmds
        out.extend_from_slice(&0u32.to_le_bytes()); // flags
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved
        out.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        out.extend_from_slice(&24u32.to_le_bytes()); // cmdsize
        out.extend_from_slice(&PLATFORM_MACOS.to_le_bytes());
        out.extend_from_slice(&minos.to_le_bytes());
        out.extend_from_slice(&sdk.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // ntools
        out
    }

    #[test]
    fn reads_the_deployment_target_and_sdk_from_lc_build_version() {
        let image = build_version_image(packed(14, 5, 0), packed(15, 2, 0));
        let floor = scan_image(&image);
        assert_eq!(floor.deployment_target.unwrap().to_string(), "14.5");
        assert_eq!(floor.sdk.unwrap().to_string(), "15.2");
    }

    #[test]
    fn reads_the_older_version_min_load_command() {
        let mut image = build_version_image(0, 0);
        // Rewrite the single command as LC_VERSION_MIN_MACOSX (version, sdk).
        let cmd = 32;
        image[cmd..cmd + 4].copy_from_slice(&LC_VERSION_MIN_MACOSX.to_le_bytes());
        image[cmd + 4..cmd + 8].copy_from_slice(&16u32.to_le_bytes());
        image[cmd + 8..cmd + 12].copy_from_slice(&packed(12, 0, 0).to_le_bytes());
        image[cmd + 12..cmd + 16].copy_from_slice(&packed(13, 1, 0).to_le_bytes());
        let floor = scan_image(&image);
        assert_eq!(floor.deployment_target.unwrap().to_string(), "12.0");
        assert_eq!(floor.sdk.unwrap().to_string(), "13.1");
    }

    /// The runtime is bounded by its *highest* minimum: one library needing
    /// 15.0 makes the whole artifact unloadable on 14.5.
    #[test]
    fn the_floor_is_the_maximum_across_binaries() {
        let mut floor = MacosFloor::default();
        floor.absorb(scan_image(&build_version_image(
            packed(14, 5, 0),
            packed(15, 2, 0),
        )));
        floor.absorb(scan_image(&build_version_image(
            packed(15, 0, 0),
            packed(15, 0, 0),
        )));
        assert_eq!(floor.deployment_target.unwrap().to_string(), "15.0");
        assert_eq!(floor.sdk.unwrap().to_string(), "15.2");
    }

    #[test]
    fn a_universal_binary_reports_the_highest_slice() {
        let thin_a = build_version_image(packed(13, 0, 0), packed(14, 0, 0));
        let thin_b = build_version_image(packed(14, 5, 0), packed(15, 2, 0));
        let header = 8 + 2 * 20;
        let offset_a = header;
        let offset_b = header + thin_a.len();

        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        fat.extend_from_slice(&2u32.to_be_bytes());
        for (offset, thin) in [(offset_a, &thin_a), (offset_b, &thin_b)] {
            fat.extend_from_slice(&0x0100_000cu32.to_be_bytes()); // cputype
            fat.extend_from_slice(&0u32.to_be_bytes()); // cpusubtype
            fat.extend_from_slice(&(offset as u32).to_be_bytes());
            fat.extend_from_slice(&(thin.len() as u32).to_be_bytes());
            fat.extend_from_slice(&0u32.to_be_bytes()); // align
        }
        fat.extend_from_slice(&thin_a);
        fat.extend_from_slice(&thin_b);

        let floor = scan_image(&fat);
        assert_eq!(floor.deployment_target.unwrap().to_string(), "14.5");
        assert_eq!(floor.sdk.unwrap().to_string(), "15.2");
    }

    /// The real shape: a universal binary's second slice starts megabytes into
    /// the file, past the header window one read pulls in. Scanning only what
    /// that window contained would take the maximum over a subset and report a
    /// floor lower than the artifact's — the one direction this must never fail
    /// in, since a too-low floor is what lets an unloadable runtime ship.
    #[test]
    fn a_slice_past_the_header_window_is_still_measured() {
        let thin_a = build_version_image(packed(13, 0, 0), packed(14, 0, 0));
        let thin_b = build_version_image(packed(14, 5, 0), packed(15, 2, 0));
        let offset_a = 8 + 2 * 20;
        let offset_b = 2 * MAX_LOAD_COMMANDS;

        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        fat.extend_from_slice(&2u32.to_be_bytes());
        for (offset, thin) in [(offset_a, &thin_a), (offset_b, &thin_b)] {
            fat.extend_from_slice(&0x0100_000cu32.to_be_bytes()); // cputype
            fat.extend_from_slice(&0u32.to_be_bytes()); // cpusubtype
            fat.extend_from_slice(&(offset as u32).to_be_bytes());
            fat.extend_from_slice(&(thin.len() as u32).to_be_bytes());
            fat.extend_from_slice(&0u32.to_be_bytes()); // align
        }
        fat.resize(offset_a, 0);
        fat.extend_from_slice(&thin_a);
        fat.resize(offset_b, 0);
        fat.extend_from_slice(&thin_b);

        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("ost-macho-fat-{}", std::process::id()));
        std::fs::write(path.as_std_path(), &fat).unwrap();
        let floor = max_macos_floor([path.as_path()]).unwrap();
        let _ = std::fs::remove_file(path.as_std_path());

        assert_eq!(floor.deployment_target.unwrap().to_string(), "14.5");
        assert_eq!(floor.sdk.unwrap().to_string(), "15.2");
        // The in-memory path sees the same thing when it has the whole image.
        assert_eq!(scan_image(&fat), floor);
    }

    /// A 64-bit fat table (8-byte offsets in 32-byte entries) reads the same.
    #[test]
    fn a_fat_64_table_is_read() {
        let thin = build_version_image(packed(15, 0, 0), packed(15, 4, 0));
        let offset = 8 + 32usize;

        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC_64.to_be_bytes());
        fat.extend_from_slice(&1u32.to_be_bytes());
        fat.extend_from_slice(&0x0100_000cu32.to_be_bytes()); // cputype
        fat.extend_from_slice(&0u32.to_be_bytes()); // cpusubtype
        fat.extend_from_slice(&(offset as u64).to_be_bytes());
        fat.extend_from_slice(&(thin.len() as u64).to_be_bytes());
        fat.extend_from_slice(&0u32.to_be_bytes()); // align
        fat.extend_from_slice(&0u32.to_be_bytes()); // reserved
        fat.extend_from_slice(&thin);

        let floor = scan_image(&fat);
        assert_eq!(floor.deployment_target.unwrap().to_string(), "15.0");
        assert_eq!(floor.sdk.unwrap().to_string(), "15.4");
    }

    /// A macOS framework bundle links whole directories
    /// (`Versions/Current -> A`), and `stage_files` keeps those in-tree links.
    /// Opening one succeeds on Unix and fails only at the first read, with
    /// EISDIR, which used to abort the measurement for every imaging leaf.
    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_contributes_nothing() {
        let root = Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("ost-macho-framework-{}", std::process::id()));
        let versions = root.join("Versions");
        let real = versions.join("A");
        std::fs::create_dir_all(real.as_std_path()).unwrap();
        let link = versions.join("Current");
        let _ = std::fs::remove_file(link.as_std_path());

        std::os::unix::fs::symlink("A", link.as_std_path()).unwrap();

        let floor = max_macos_floor([link.as_path(), real.as_path()]);
        let _ = std::fs::remove_dir_all(root.as_std_path());

        let floor = floor.expect("a linked directory is skipped, not an error");
        assert!(floor.is_empty());
    }

    #[test]
    fn a_non_macho_file_contributes_nothing() {
        assert!(scan_image(b"#!/bin/sh\necho hello\n").is_empty());
        assert!(scan_image(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]).is_empty());
        assert!(scan_image(&[]).is_empty());
    }

    /// A corrupt `cmdsize` must terminate the walk rather than spin or read
    /// past the buffer.
    #[test]
    fn a_corrupt_command_size_stops_the_walk() {
        let mut image = build_version_image(packed(14, 5, 0), packed(15, 2, 0));
        image[16..20].copy_from_slice(&64u32.to_le_bytes()); // ncmds = 64
        image[36..40].copy_from_slice(&0u32.to_le_bytes()); // cmdsize = 0
        assert!(scan_image(&image).is_empty());
    }
}
