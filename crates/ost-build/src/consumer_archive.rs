// SPDX-License-Identifier: Apache-2.0
//! Deterministic ecosystem archive writers used by derived consumer packages.
//!
//! Wheels are ZIP files and npm packages are gzip-compressed tar archives. The
//! writers deliberately normalize timestamps, ownership and entry ordering so
//! registry routing metadata does not introduce ambient producer state.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, Write};

use camino::{Utf8Path, Utf8PathBuf};
use flate2::{Compression, GzBuilder};

use ost_core::digest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerArchiveEntry {
    /// Regular file on disk.
    pub source: Utf8PathBuf,
    /// Portable path inside the archive, without a leading slash.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerArchiveResult {
    pub archive_digest: String,
    pub archive_size: u64,
    pub files: usize,
}

impl ConsumerArchiveEntry {
    pub fn new(source: Utf8PathBuf, path: impl Into<String>) -> io::Result<Self> {
        let path = path.into().replace('\\', "/");
        validate_archive_path(&path)?;
        if !source.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("consumer archive input is not a regular file: {source}"),
            ));
        }
        Ok(Self { source, path })
    }
}

fn validate_archive_path(path: &str) -> io::Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsafe consumer archive path: {path}"),
        ));
    }
    Ok(())
}

fn sorted_entries(entries: &[ConsumerArchiveEntry]) -> io::Result<Vec<&ConsumerArchiveEntry>> {
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));
    if sorted.windows(2).any(|pair| pair[0].path == pair[1].path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "consumer archive contains duplicate paths",
        ));
    }
    Ok(sorted)
}

fn result(path: &Utf8Path, files: usize) -> io::Result<ConsumerArchiveResult> {
    let mut file = File::open(path)?;
    let (archive_digest, archive_size) = digest::sha256_hex_reader(&mut file)?;
    Ok(ConsumerArchiveResult {
        archive_digest,
        archive_size,
        files,
    })
}

/// Render the wheel `RECORD` file for all current entries and the record itself.
pub fn wheel_record(entries: &[ConsumerArchiveEntry], record_path: &str) -> io::Result<String> {
    validate_archive_path(record_path)?;
    let sorted = sorted_entries(entries)?;
    let mut output = String::new();
    for entry in sorted {
        let mut file = File::open(&entry.source)?;
        let (sha256, size) = digest::sha256_hex_reader(&mut file)?;
        let bytes = decode_hex(sha256.strip_prefix("sha256:").unwrap_or_default())?;
        output.push_str(&entry.path);
        output.push_str(",sha256=");
        output.push_str(&base64_url_no_pad(&bytes));
        output.push(',');
        output.push_str(&size.to_string());
        output.push('\n');
    }
    output.push_str(record_path);
    output.push_str(",,\n");
    Ok(output)
}

fn decode_hex(value: &str) -> io::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "odd hex length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            digit(pair[0])
                .zip(digit(pair[1]))
                .map(|(high, low)| high << 4 | low)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid hex"))
        })
        .collect()
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[((value >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(TABLE[(value & 63) as usize] as char);
        }
    }
    output
}

#[derive(Debug)]
struct ZipEntry {
    path: String,
    crc32: u32,
    size: u64,
    offset: u64,
}

fn write_u16(writer: &mut impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn zip64_extra(size: u64, offset: Option<u64>) -> Vec<u8> {
    let mut extra = Vec::with_capacity(if offset.is_some() { 28 } else { 20 });
    extra.extend_from_slice(&1u16.to_le_bytes());
    extra.extend_from_slice(&(if offset.is_some() { 24u16 } else { 16u16 }).to_le_bytes());
    extra.extend_from_slice(&size.to_le_bytes());
    extra.extend_from_slice(&size.to_le_bytes());
    if let Some(offset) = offset {
        extra.extend_from_slice(&offset.to_le_bytes());
    }
    extra
}

fn zip64_central_extra(size: u64, size64: bool, offset: u64, offset64: bool) -> Vec<u8> {
    let body_len = usize::from(size64) * 16 + usize::from(offset64) * 8;
    let mut extra = Vec::with_capacity(body_len + 4);
    extra.extend_from_slice(&1u16.to_le_bytes());
    extra.extend_from_slice(&(body_len as u16).to_le_bytes());
    if size64 {
        extra.extend_from_slice(&size.to_le_bytes());
        extra.extend_from_slice(&size.to_le_bytes());
    }
    if offset64 {
        extra.extend_from_slice(&offset.to_le_bytes());
    }
    extra
}

/// Pack a deterministic, uncompressed ZIP suitable for a Python wheel.
///
/// Runtime archives are already compressed, so storing entries avoids a costly
/// and ineffective second compression pass. ZIP64 is emitted when required.
pub fn pack_wheel(
    entries: &[ConsumerArchiveEntry],
    destination: &Utf8Path,
) -> io::Result<ConsumerArchiveResult> {
    let entries = sorted_entries(entries)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(destination)?;
    let mut writer = BufWriter::new(file);
    let mut central = Vec::with_capacity(entries.len());

    for entry in &entries {
        let name = entry.path.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "ZIP entry name is too long")
        })?;
        let mut source = File::open(&entry.source)?;
        let mut crc = crc32fast::Hasher::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            size += read as u64;
        }
        let crc32 = crc.finalize();
        let offset = writer.stream_position()?;
        let zip64 = size > u32::MAX as u64;
        let extra = if zip64 {
            zip64_extra(size, None)
        } else {
            Vec::new()
        };

        write_u32(&mut writer, 0x0403_4b50)?;
        write_u16(&mut writer, if zip64 { 45 } else { 20 })?;
        write_u16(&mut writer, 0x0800)?; // UTF-8 names
        write_u16(&mut writer, 0)?; // stored
        write_u16(&mut writer, 0)?; // deterministic DOS time
        write_u16(&mut writer, 0x21)?; // 1980-01-01
        write_u32(&mut writer, crc32)?;
        write_u32(&mut writer, if zip64 { u32::MAX } else { size as u32 })?;
        write_u32(&mut writer, if zip64 { u32::MAX } else { size as u32 })?;
        write_u16(&mut writer, name_len)?;
        write_u16(&mut writer, extra.len() as u16)?;
        writer.write_all(name)?;
        writer.write_all(&extra)?;
        source.rewind()?;
        io::copy(&mut source, &mut writer)?;
        central.push(ZipEntry {
            path: entry.path.clone(),
            crc32,
            size,
            offset,
        });
    }

    let central_offset = writer.stream_position()?;
    for entry in &central {
        let name = entry.path.as_bytes();
        let size64 = entry.size > u32::MAX as u64;
        let offset64 = entry.offset > u32::MAX as u64;
        let extra = if size64 || offset64 {
            zip64_central_extra(entry.size, size64, entry.offset, offset64)
        } else {
            Vec::new()
        };
        write_u32(&mut writer, 0x0201_4b50)?;
        write_u16(&mut writer, 0x031e)?; // Unix, ZIP 3.0
        write_u16(&mut writer, if size64 || offset64 { 45 } else { 20 })?;
        write_u16(&mut writer, 0x0800)?;
        write_u16(&mut writer, 0)?;
        write_u16(&mut writer, 0)?;
        write_u16(&mut writer, 0x21)?;
        write_u32(&mut writer, entry.crc32)?;
        write_u32(
            &mut writer,
            if size64 { u32::MAX } else { entry.size as u32 },
        )?;
        write_u32(
            &mut writer,
            if size64 { u32::MAX } else { entry.size as u32 },
        )?;
        write_u16(&mut writer, name.len() as u16)?;
        write_u16(&mut writer, extra.len() as u16)?;
        write_u16(&mut writer, 0)?;
        write_u16(&mut writer, 0)?;
        write_u16(&mut writer, 0)?;
        write_u32(&mut writer, 0o100644 << 16)?;
        write_u32(
            &mut writer,
            if offset64 {
                u32::MAX
            } else {
                entry.offset as u32
            },
        )?;
        writer.write_all(name)?;
        writer.write_all(&extra)?;
    }
    let central_size = writer.stream_position()? - central_offset;
    let zip64 = central.len() > u16::MAX as usize
        || central_offset > u32::MAX as u64
        || central_size > u32::MAX as u64
        || central
            .iter()
            .any(|entry| entry.size > u32::MAX as u64 || entry.offset > u32::MAX as u64);
    if zip64 {
        let zip64_offset = writer.stream_position()?;
        write_u32(&mut writer, 0x0606_4b50)?;
        write_u64(&mut writer, 44)?;
        write_u16(&mut writer, 0x031e)?;
        write_u16(&mut writer, 45)?;
        write_u32(&mut writer, 0)?;
        write_u32(&mut writer, 0)?;
        write_u64(&mut writer, central.len() as u64)?;
        write_u64(&mut writer, central.len() as u64)?;
        write_u64(&mut writer, central_size)?;
        write_u64(&mut writer, central_offset)?;
        write_u32(&mut writer, 0x0706_4b50)?;
        write_u32(&mut writer, 0)?;
        write_u64(&mut writer, zip64_offset)?;
        write_u32(&mut writer, 1)?;
    }
    write_u32(&mut writer, 0x0605_4b50)?;
    write_u16(&mut writer, 0)?;
    write_u16(&mut writer, 0)?;
    let count = u16::try_from(central.len()).unwrap_or(u16::MAX);
    write_u16(&mut writer, count)?;
    write_u16(&mut writer, count)?;
    write_u32(&mut writer, u32::try_from(central_size).unwrap_or(u32::MAX))?;
    write_u32(
        &mut writer,
        u32::try_from(central_offset).unwrap_or(u32::MAX),
    )?;
    write_u16(&mut writer, 0)?;
    writer.flush()?;
    drop(writer);
    result(destination, central.len())
}

/// Pack a deterministic npm-compatible `package/` tarball.
pub fn pack_npm_tgz(
    entries: &[ConsumerArchiveEntry],
    destination: &Utf8Path,
) -> io::Result<ConsumerArchiveResult> {
    let entries = sorted_entries(entries)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = File::create(destination)?;
    let encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(output, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    for entry in &entries {
        let mut source = File::open(&entry.source)?;
        let size = source.metadata()?.len();
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(&mut header, format!("package/{}", entry.path), &mut source)?;
    }
    let encoder = archive.into_inner()?;
    encoder.finish()?;
    result(destination, entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_zip_paths(bytes: &[u8]) -> Vec<String> {
        let mut cursor = 0usize;
        let mut paths = Vec::new();
        let u16_at =
            |offset: usize| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
        let u32_at =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        while bytes.get(cursor..cursor + 4) == Some(&0x0403_4b50u32.to_le_bytes()) {
            let size = u32_at(cursor + 18) as usize;
            let name_len = u16_at(cursor + 26) as usize;
            let extra_len = u16_at(cursor + 28) as usize;
            assert_ne!(size, u32::MAX as usize, "fixture should not need ZIP64");
            let name_start = cursor + 30;
            paths.push(
                std::str::from_utf8(&bytes[name_start..name_start + name_len])
                    .unwrap()
                    .to_string(),
            );
            cursor = name_start + name_len + extra_len + size;
        }
        assert_eq!(u32_at(cursor), 0x0201_4b50);
        paths
    }

    fn scratch(name: &str) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "ost-consumer-archive-{name}-{}",
                std::process::id()
            ))
    }

    #[test]
    fn archives_are_deterministic_and_record_is_url_safe() {
        let root = scratch("deterministic");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), b"alpha\n").unwrap();
        std::fs::write(root.join("b.txt"), b"beta\n").unwrap();
        let entries = vec![
            ConsumerArchiveEntry::new(root.join("b.txt"), "b.txt").unwrap(),
            ConsumerArchiveEntry::new(root.join("a.txt"), "a.txt").unwrap(),
        ];
        let record = wheel_record(&entries, "x.dist-info/RECORD").unwrap();
        assert!(record.contains("a.txt,sha256="));
        assert!(!record.lines().any(|line| line
            .split(',')
            .nth(1)
            .is_some_and(|digest| digest.ends_with('='))));

        let first = pack_wheel(&entries, &root.join("first.whl")).unwrap();
        let second = pack_wheel(&entries, &root.join("second.whl")).unwrap();
        assert_eq!(first.archive_digest, second.archive_digest);
        assert_eq!(
            stored_zip_paths(&std::fs::read(root.join("first.whl")).unwrap()),
            ["a.txt", "b.txt"]
        );
        let first = pack_npm_tgz(&entries, &root.join("first.tgz")).unwrap();
        let second = pack_npm_tgz(&entries, &root.join("second.tgz")).unwrap();
        assert_eq!(first.archive_digest, second.archive_digest);
        let decoder = flate2::read::GzDecoder::new(File::open(root.join("first.tgz")).unwrap());
        let mut archive = tar::Archive::new(decoder);
        let paths = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, ["package/a.txt", "package/b.txt"]);
        std::fs::remove_dir_all(root).ok();
    }
}
