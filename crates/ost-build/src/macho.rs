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

use std::io::{self, Read};

use camino::Utf8Path;

/// 64-bit Mach-O, little-endian (`MH_MAGIC_64`). Every macOS target OpenStrata
/// supports is 64-bit and little-endian; a 32-bit or byte-swapped image is not
/// a runtime binary we produce and contributes nothing.
const MACHO_MAGIC_64: u32 = 0xfeed_facf;

/// Universal ("fat") binary magic, which is big-endian by definition.
const FAT_MAGIC: u32 = 0xcafe_babe;

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
    let mut file = match std::fs::File::open(path.as_std_path()) {
        Ok(file) => file,
        // A dangling symlink or a file that vanished between staging and scan is
        // not a Mach-O binary; do not fail the whole measurement over it.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MacosFloor::default()),
        Err(error) => return Err(error),
    };
    // The header plus its load commands is all we need, and it is at the front
    // of the file — a multi-hundred-megabyte library is never read whole.
    let mut head = Vec::new();
    file.by_ref()
        .take(MAX_LOAD_COMMANDS as u64)
        .read_to_end(&mut head)?;
    Ok(scan_image(&head))
}

/// Scan a Mach-O (or universal) image prefix for its deployment floor.
fn scan_image(bytes: &[u8]) -> MacosFloor {
    match read_u32_be(bytes, 0) {
        // A universal binary holds several images; each slice's own header
        // carries the floor, and the runtime is bounded by the highest.
        Some(FAT_MAGIC) => {
            let mut floor = MacosFloor::default();
            let Some(count) = read_u32_be(bytes, 4) else {
                return floor;
            };
            for index in 0..count.min(64) as usize {
                let entry = 8 + index * 20;
                let Some(offset) = read_u32_be(bytes, entry + 8) else {
                    break;
                };
                let offset = offset as usize;
                if offset < bytes.len() {
                    floor.absorb(scan_thin(&bytes[offset..]));
                }
            }
            floor
        }
        _ => scan_thin(bytes),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
