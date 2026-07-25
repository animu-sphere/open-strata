// SPDX-License-Identifier: Apache-2.0
//! Family validators: turn a candidate root into a validated record, or a
//! rejection that says why.
//!
//! Validation is **read-only, bounded, and never interactive**. It reads files
//! the vendor ships, lists a few directories, and — only when explicitly asked
//! — runs the host's own `--version` under a deadline. It never sources shell
//! setup, never launches a GUI, never writes inside a host install, and
//! isolates its failures so one unreadable candidate cannot sink a scan.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use camino::Utf8Path;
use ost_core::host::Os;

use crate::record::{
    HostExecutable, HostFamily, HostPython, HostRejection, HostVersion, VersionSource,
};

/// Default ceiling on a version probe. A host that has not answered `--version`
/// in this long is waiting for something — a license server, a display — that
/// discovery must not wait for.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How much a validator may do.
#[derive(Debug, Clone)]
pub struct ValidateOptions {
    /// Run the host's own executable to resolve a version metadata could not.
    /// Off by default: discovery does not execute third-party binaries unless
    /// the operator asks it to.
    pub probe: bool,
    pub probe_timeout: Duration,
    /// The machine being validated against, injected so tests do not depend on
    /// the OS they happen to run on.
    pub os: Os,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        ValidateOptions {
            probe: false,
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
            os: Os::current(),
        }
    }
}

/// What a validator confirmed about an install.
#[derive(Debug, Clone)]
pub struct HostValidation {
    pub version: Option<HostVersion>,
    pub executables: BTreeMap<String, HostExecutable>,
    pub python: Option<HostPython>,
    pub markers: Vec<String>,
}

/// One executable a family is known to ship, by role.
///
/// `relative` is borrowed when the layout is constant and owned only where the
/// executable suffix varies, so building the table allocates at most a handful
/// of short strings and leaks none of them.
struct ExecutableSpec {
    role: &'static str,
    relative: Cow<'static, str>,
    /// An anchor is a name only this product ships; finding one is what makes
    /// a directory an install rather than a directory.
    anchor: bool,
}

/// Confirm (or refuse) a candidate root as an install of `family`.
pub fn validate(
    family: HostFamily,
    root: &Utf8Path,
    options: &ValidateOptions,
) -> Result<HostValidation, HostRejection> {
    if !root.as_std_path().is_dir() {
        return Err(HostRejection {
            code: "HOST_ROOT_UNREADABLE".into(),
            message: format!("'{root}' is not a readable directory"),
        });
    }

    let specs = executable_specs(family, options.os);
    let mut executables = BTreeMap::new();
    let mut found_anchor = false;
    for spec in &specs {
        let path = root.join(spec.relative.as_ref());
        let Ok(metadata) = std::fs::metadata(path.as_std_path()) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        found_anchor |= spec.anchor;
        executables.insert(
            spec.role.to_string(),
            HostExecutable {
                path: spec.relative.to_string(),
                size: metadata.len(),
                modified_unix: modified_unix(&metadata),
            },
        );
    }

    if !found_anchor {
        let looked_for = specs
            .iter()
            .filter(|spec| spec.anchor)
            .map(|spec| spec.relative.as_ref())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HostRejection {
            code: "HOST_MARKERS_MISSING".into(),
            message: format!(
                "'{root}' holds no {} executable (looked for: {looked_for})",
                family.product()
            ),
        });
    }

    let markers = present_markers(family, root);
    let version = resolve_version(family, root, &executables, options);
    let python = resolve_python(family, root, options.os);

    Ok(HostValidation {
        version,
        executables,
        python,
        markers,
    })
}

fn executable_specs(family: HostFamily, os: Os) -> Vec<ExecutableSpec> {
    let exe = if os == Os::Windows { ".exe" } else { "" };
    match family {
        HostFamily::Maya => {
            let mut specs = vec![ExecutableSpec {
                role: "interpreter",
                relative: format!("bin/mayapy{exe}").into(),
                anchor: true,
            }];
            // macOS keeps the interactive binary inside the app bundle while
            // the interpreter stays in `bin/`, so the two are not siblings.
            if os == Os::Macos {
                specs.push(ExecutableSpec {
                    role: "interactive",
                    relative: "Maya.app/Contents/MacOS/Maya".into(),
                    anchor: true,
                });
            } else {
                specs.push(ExecutableSpec {
                    role: "interactive",
                    relative: format!("bin/maya{exe}").into(),
                    anchor: true,
                });
            }
            if os == Os::Windows {
                // Only Windows ships a separate batch binary; elsewhere batch
                // is `maya -batch`, which is the interactive executable.
                specs.push(ExecutableSpec {
                    role: "batch",
                    relative: "bin/mayabatch.exe".into(),
                    anchor: false,
                });
            }
            specs.push(ExecutableSpec {
                role: "render",
                relative: format!("bin/Render{exe}").into(),
                anchor: false,
            });
            specs
        }
        HostFamily::Houdini => {
            // macOS keeps the toolset inside a versioned framework, so `$HFS`
            // there sits several levels below the directory an operator would
            // call the install root. Both shapes are accepted — a bundle root
            // and an `$HFS` root are the same install named differently — and
            // the recorded relative path says which one was found.
            let mut bins = vec!["bin"];
            if os == Os::Macos {
                bins.insert(
                    0,
                    "Frameworks/Houdini.framework/Versions/Current/Resources/bin",
                );
            }
            let mut specs = Vec::new();
            for bin in bins {
                specs.extend([
                    ExecutableSpec {
                        role: "interpreter",
                        relative: format!("{bin}/hython{exe}").into(),
                        anchor: true,
                    },
                    ExecutableSpec {
                        role: "interactive",
                        relative: format!("{bin}/houdini{exe}").into(),
                        anchor: true,
                    },
                    ExecutableSpec {
                        role: "render",
                        relative: format!("{bin}/husk{exe}").into(),
                        anchor: false,
                    },
                ]);
            }
            specs
        }
    }
}

/// Directories the vendor ships that corroborate the identification. Recorded
/// as evidence and folded into the fingerprint, so a layout change is visible.
fn present_markers(family: HostFamily, root: &Utf8Path) -> Vec<String> {
    let candidates: &[&str] = match family {
        HostFamily::Maya => &["modules", "plug-ins", "scripts", "icons", "include/maya"],
        HostFamily::Houdini => &[
            "houdini",
            "toolkit",
            "packages",
            "dsolib",
            "Frameworks/Houdini.framework",
        ],
    };
    let mut markers: Vec<String> = candidates
        .iter()
        .filter(|relative| root.join(relative).as_std_path().is_dir())
        .map(|relative| (*relative).to_string())
        .collect();
    markers.sort();
    markers
}

/// Resolve a version least-invasively: vendor metadata, then an opt-in probe,
/// then the directory name — recording which rung answered.
fn resolve_version(
    family: HostFamily,
    root: &Utf8Path,
    executables: &BTreeMap<String, HostExecutable>,
    options: &ValidateOptions,
) -> Option<HostVersion> {
    if let Some(version) = version_from_metadata(family, root) {
        return Some(version);
    }
    if options.probe {
        if let Some(version) = version_from_probe(family, root, executables, options.probe_timeout)
        {
            return Some(version);
        }
    }
    version_from_directory_name(family, root)
}

/// Read a version out of a file the vendor ships.
///
/// Maya's `MAYA_APP_VERSION` lives in the devkit header, which recent releases
/// ship separately — so this is a high-confidence answer when it is there and
/// simply absent when it is not, rather than something to fake.
fn version_from_metadata(family: HostFamily, root: &Utf8Path) -> Option<HostVersion> {
    match family {
        HostFamily::Maya => {
            let header = read_bounded(&root.join("include/maya/MTypes.h"))?;
            if let Some(raw) = define_value(&header, "MAYA_APP_VERSION") {
                return HostVersion::from_raw(raw.trim(), VersionSource::Metadata);
            }
            // `MAYA_API_VERSION` is `YYYYRRMM`-shaped; its leading four digits
            // are the application year.
            let api = define_value(&header, "MAYA_API_VERSION")?;
            let year = api.trim().get(..4)?;
            HostVersion::from_raw(year, VersionSource::Metadata)
        }
        HostFamily::Houdini => {
            let header = read_bounded(&root.join("toolkit/include/SYS/SYS_Version.h"))?;
            let raw = define_value(&header, "SYS_VERSION_FULL")?;
            HostVersion::from_raw(raw.trim().trim_matches('"'), VersionSource::Metadata)
        }
    }
}

/// Detect the host's embedded Python from its shipped layout.
fn resolve_python(family: HostFamily, root: &Utf8Path, os: Os) -> Option<HostPython> {
    // Ordered most- to least-specific; the first hit wins so a corroborating
    // directory cannot override a definitive one. Houdini's
    // `houdini/python3.11libs` names the ABI outright, which is why it leads.
    let directories: &[&str] = match family {
        HostFamily::Maya => &["lib", "Frameworks/Python.framework/Versions"],
        HostFamily::Houdini => &[
            "houdini",
            ".",
            "python/lib",
            "Frameworks/Houdini.framework/Versions/Current/Resources/houdini",
        ],
    };
    for directory in directories {
        if let Some(python) = python_from_directory(root, directory) {
            return Some(python);
        }
    }
    if os == Os::Windows {
        // Maya ships the interpreter as `bin/python311.dll` rather than a
        // versioned directory.
        return python_from_windows_runtime(root);
    }
    None
}

/// The value of `#define <name> <value>` in a C header, ignoring comments.
fn define_value(source: &str, name: &str) -> Option<String> {
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("#define") else {
            continue;
        };
        let mut parts = rest.trim().splitn(2, char::is_whitespace);
        if parts.next()? != name {
            continue;
        }
        let value = parts.next()?.trim();
        let value = value.split("//").next().unwrap_or(value);
        let value = value.split("/*").next().unwrap_or(value);
        return Some(value.trim().to_string());
    }
    None
}

/// Read a metadata file, refusing anything implausibly large so a wrong path
/// cannot turn validation into a long read.
fn read_bounded(path: &Utf8Path) -> Option<String> {
    const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
    let metadata = std::fs::metadata(path.as_std_path()).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        return None;
    }
    std::fs::read_to_string(path.as_std_path()).ok()
}

/// Ask the host itself, under a deadline.
fn version_from_probe(
    family: HostFamily,
    root: &Utf8Path,
    executables: &BTreeMap<String, HostExecutable>,
    timeout: Duration,
) -> Option<HostVersion> {
    let (role, args) = match family {
        // `maya -v` prints the product banner and exits without a GUI.
        HostFamily::Maya => ("interactive", vec!["-v"]),
        // Houdini's interpreter reports *Python's* version — `hython --version`
        // answers `Python 3.11.7`, which would be a confidently wrong host
        // version. `husk --version` names the Houdini build and needs no
        // license or display.
        HostFamily::Houdini => ("render", vec!["--version"]),
    };
    let executable = executables.get(role)?.absolute(root);
    let output = run_bounded(&executable, &args, timeout)?;
    let raw = probe_version_text(family, &output)?;
    HostVersion::from_raw(&raw, VersionSource::Probe)
}

/// Extract the version from a host's own version banner.
///
/// Split out from the spawn so the parsing contract is testable without a real
/// DCC install: the banners are stable, the spawn is not reproducible in CI.
pub(crate) fn probe_version_text(family: HostFamily, output: &str) -> Option<String> {
    // Any of these identifies the line as the product speaking. Houdini needs
    // more than its own name because `husk` prints its own path, which on Linux
    // reads `/opt/hfs20.5.550/bin/husk version 20.5.550`.
    let wanted: &[&str] = match family {
        HostFamily::Maya => &["maya"],
        HostFamily::Houdini => &["houdini", "husk", "hfs"],
    };
    for line in output.lines() {
        let lowered = line.to_lowercase();
        if !wanted.iter().any(|name| lowered.contains(name)) {
            continue;
        }
        // Anchor on the literal word `version` when the banner has one: it is
        // the one part of the line a directory full of digits cannot displace.
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(index) = words
            .iter()
            .position(|word| word.eq_ignore_ascii_case("version"))
        {
            if let Some(version) = words.get(index + 1).and_then(|word| numeric_token(word)) {
                return Some(version);
            }
        }
        // Otherwise the first number of two or more digits: `Maya 2024, Cut
        // Number 202310181224` gives `2024`.
        for token in line.split(|character: char| !character.is_ascii_digit() && character != '.') {
            if let Some(version) = numeric_token(token) {
                return Some(version);
            }
        }
    }
    None
}

/// A dotted-or-plain number of at least two digits, trimmed of punctuation.
fn numeric_token(value: &str) -> Option<String> {
    let token = value.trim_matches(|character: char| !character.is_ascii_digit());
    if token.len() < 2 {
        return None;
    }
    token
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
        .then(|| token.to_string())
}

/// Run a command with its output captured and a hard deadline, killing it if
/// the deadline passes. `--version` does not spawn a tree, so terminating the
/// direct child is sufficient here.
fn run_bounded(program: &Utf8Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program.as_std_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    // Some hosts print their banner on stderr; both are the product speaking.
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}

/// The last resort: read the install directory's name.
fn version_from_directory_name(family: HostFamily, root: &Utf8Path) -> Option<HostVersion> {
    let prefixes: &[&str] = match family {
        HostFamily::Maya => &["maya"],
        HostFamily::Houdini => &["houdini", "hfs"],
    };
    // The install directory usually carries the version (`maya2026`,
    // `hfs20.5.550`); a site that versions one level up (`/tools/maya/2025.3`)
    // is already covered because that directory *is* the root. The parent is
    // the fallback for roots that name a flavor instead (`.../2025.3/x64`).
    let names = [
        root.file_name(),
        root.parent().and_then(Utf8Path::file_name),
    ];
    for name in names.into_iter().flatten() {
        let mut candidate = name.to_lowercase();
        for prefix in prefixes {
            if let Some(rest) = candidate.strip_prefix(prefix) {
                candidate = rest.to_string();
                break;
            }
        }
        let candidate = candidate.trim_start_matches([' ', '-', '_', '.']);
        let digits: String = candidate
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect();
        let digits = digits.trim_end_matches('.');
        if digits.is_empty() {
            continue;
        }
        if let Some(version) = HostVersion::from_raw(digits, VersionSource::DirectoryName) {
            return Some(version);
        }
    }
    None
}

/// Find `python3.11`, `python3.11libs`, `python313`, or a bare `3.11` entry.
fn python_from_directory(root: &Utf8Path, relative: &str) -> Option<HostPython> {
    let directory = root.join(relative);
    let entries = std::fs::read_dir(directory.as_std_path()).ok()?;
    let mut best: Option<(u32, u32, String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // The concatenated form (`python313`) is only unambiguous behind the
        // `python` prefix: a bare `550` directory is a build number, not an ABI.
        let (stripped, concatenated) = match name.strip_prefix("python") {
            Some(rest) => (rest.trim_end_matches("libs"), true),
            None => (name.as_str(), false),
        };
        let Some((major, minor)) = parse_major_minor(stripped, concatenated) else {
            continue;
        };
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let evidence = if relative == "." {
            name
        } else {
            format!("{relative}/{name}")
        };
        // A host can ship more than one (a legacy `python2.7` beside the real
        // one); the newest is the one it runs.
        if best
            .as_ref()
            .is_none_or(|(best_major, best_minor, _)| (major, minor) > (*best_major, *best_minor))
        {
            best = Some((major, minor, evidence));
        }
    }
    let (major, minor, evidence) = best?;
    HostPython::from_layout(&format!("{major}.{minor}"), evidence)
}

/// `bin/python311.dll` → 3.11.
fn python_from_windows_runtime(root: &Utf8Path) -> Option<HostPython> {
    let entries = std::fs::read_dir(root.join("bin").as_std_path()).ok()?;
    let mut best: Option<(u32, u32, String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lowered = name.to_lowercase();
        let Some(digits) = lowered
            .strip_prefix("python")
            .and_then(|rest| rest.strip_suffix(".dll"))
        else {
            continue;
        };
        let Some((major, minor)) = parse_major_minor(digits, true) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(best_major, best_minor, _)| (major, minor) > (*best_major, *best_minor))
        {
            best = Some((major, minor, format!("bin/{name}")));
        }
    }
    let (major, minor, evidence) = best?;
    HostPython::from_layout(&format!("{major}.{minor}"), evidence)
}

/// Parse a Python major/minor written either dotted (`3.11`, from
/// `python3.11libs`) or concatenated (`313`, from a `python313` directory).
///
/// Only Python 3 counts: a host that still ships a `python2.7` tree beside the
/// real interpreter must not have that reported as its ABI.
fn parse_major_minor(value: &str, concatenated: bool) -> Option<(u32, u32)> {
    if let Some((major, minor)) = value.split_once('.') {
        let major: u32 = major.parse().ok()?;
        let minor: u32 = minor.parse().ok()?;
        return (major >= 3).then_some((major, minor));
    }
    if !concatenated || value.len() < 2 || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let major: u32 = value[..1].parse().ok()?;
    let minor: u32 = value[1..].parse().ok()?;
    (major >= 3).then_some((major, minor))
}

fn modified_unix(metadata: &std::fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;
    match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).ok(),
        Err(error) => i64::try_from(error.duration().as_secs()).ok().map(|s| -s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_defines_are_read_without_their_comments() {
        let source =
            "#define MAYA_API_VERSION 20260000 // release\n#define MAYA_APP_VERSION 2026\n";
        assert_eq!(
            define_value(source, "MAYA_API_VERSION").as_deref(),
            Some("20260000")
        );
        assert_eq!(
            define_value(source, "MAYA_APP_VERSION").as_deref(),
            Some("2026")
        );
        assert_eq!(define_value(source, "MAYA_MISSING"), None);
    }

    #[test]
    fn directory_names_yield_a_low_confidence_version_or_nothing() {
        for (path, expected) in [
            ("/opt/hfs20.5.550", Some("20.5.550")),
            ("/opt/houdini20.5", Some("20.5")),
            // Both vendor spellings of a Windows/macOS install directory, and
            // the four-component build Houdini actually ships.
            ("/Applications/Houdini/Houdini20.5.332", Some("20.5.332")),
            (
                "C:/Program Files/Side Effects Software/Houdini 22.0.386.4",
                Some("22.0.386.4"),
            ),
            ("/tools/maya/2025.3", Some("2025.3")),
            ("/usr/autodesk/maya2026", Some("2026")),
        ] {
            let family = if path.contains("maya") {
                HostFamily::Maya
            } else {
                HostFamily::Houdini
            };
            let version = version_from_directory_name(family, Utf8Path::new(path));
            assert_eq!(
                version.as_ref().map(|version| version.raw.as_str()),
                expected,
                "{path}"
            );
            if let Some(version) = version {
                assert_eq!(version.confidence, "low", "{path}");
            }
        }

        // A site that renames an install has no version here, and inventing one
        // would be worse than saying so.
        assert!(version_from_directory_name(
            HostFamily::Maya,
            Utf8Path::new("/tools/maya/current")
        )
        .is_none());
    }

    #[test]
    fn probe_banners_yield_the_product_version() {
        // Both banners are verbatim from real installs (Maya 2024, Houdini
        // 20.5.522), not invented shapes.
        assert_eq!(
            probe_version_text(HostFamily::Maya, "Maya 2024, Cut Number 202310181224\n").as_deref(),
            Some("2024"),
            "the cut number must not be mistaken for the version"
        );
        assert_eq!(
            probe_version_text(
                HostFamily::Houdini,
                "C:/Program Files/Side Effects Software/Houdini 20.5.522/bin/husk.exe version 20.5.522\n"
            )
            .as_deref(),
            Some("20.5.522")
        );
        // `husk` prints its own path, so a site whose directories carry digits
        // must not have one of them read as the host version.
        assert_eq!(
            probe_version_text(
                HostFamily::Houdini,
                "/tools/2024/dcc/hfs/bin/husk version 20.5.550\n"
            )
            .as_deref(),
            Some("20.5.550")
        );
        // Nothing recognizable means no version, not a guess.
        assert_eq!(
            probe_version_text(HostFamily::Maya, "error: no license\n"),
            None
        );
        // Houdini's interpreter answers with Python's version; that line names
        // no Houdini product and must never become the host version.
        assert_eq!(
            probe_version_text(HostFamily::Houdini, "Python 3.11.7\n"),
            None
        );
    }

    #[test]
    fn python_versions_are_read_from_both_vendor_spellings() {
        // `python3.11libs` (dotted) and `python313` (concatenated) are both
        // real shipped layouts.
        assert_eq!(parse_major_minor("3.11", false), Some((3, 11)));
        assert_eq!(parse_major_minor("313", true), Some((3, 13)));
        assert_eq!(
            parse_major_minor("2.7", false),
            None,
            "a Python 2 leftover is not this host's ABI"
        );
        // A bare number is only an ABI behind the `python` prefix; otherwise a
        // build-number directory would be read as Python 5.50.
        assert_eq!(parse_major_minor("550", false), None);
        assert_eq!(parse_major_minor("3", true), None);
        let python = HostPython::from_layout("3.11", "houdini/python3.11libs").unwrap();
        assert_eq!(python.abi_tag, "cp311");
    }
}
