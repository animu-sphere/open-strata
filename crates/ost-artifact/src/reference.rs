// SPDX-License-Identifier: Apache-2.0
//! Remote artifact references (remote-artifact-transport.md, "Reference form").
//!
//! Two forms are recognized:
//!
//! ```text
//! oci://ghcr.io/owner/openstrata-runtime@sha256:<oci-manifest-digest>
//! oci://registry.example.com/vfx/openusd-runtime:usd-24.08-linux-x86_64
//! file:///abs/path/to/dist-dir
//! ```
//!
//! Tags are convenience; digests are the contract. A reference is **pinned**
//! when it carries an `@sha256:<hex>` digest — CI and the pull path require a
//! pin, while `ost artifact resolve` exists to turn a tag into one.

use ost_core::{Error, Result};

use crate::record::is_sha256_ref;

/// A parsed remote artifact reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteReference {
    Oci(OciReference),
    /// A local producer output directory (`file://`): the filesystem transport.
    File(FileReference),
}

impl RemoteReference {
    /// Parse `oci://…` / `file://…` into a reference. Anything else is a usage
    /// error — local digests are resolved by the registry, not the transport.
    pub fn parse(input: &str) -> Result<RemoteReference> {
        if let Some(rest) = input.strip_prefix("oci://") {
            return Ok(RemoteReference::Oci(OciReference::parse_rest(input, rest)?));
        }
        if let Some(rest) = input.strip_prefix("file://") {
            return Ok(RemoteReference::File(FileReference::parse_rest(
                input, rest,
            )?));
        }
        Err(Error::usage(format!(
            "'{input}' is not a remote artifact reference \
             (expected oci://<registry>/<repository>[:tag][@sha256:<digest>] or file://<dist-dir>)"
        )))
    }

    /// `true` when the reference pins an immutable digest (or, for a file
    /// reference, is inherently content-addressed at verification time).
    pub fn is_pinned(&self) -> bool {
        match self {
            RemoteReference::Oci(r) => r.digest.is_some(),
            // A file reference has no mutable name to pin; its archive digest
            // is verified against the producer manifest on every pull.
            RemoteReference::File(_) => true,
        }
    }

    /// The canonical locator string for evidence output.
    pub fn locator(&self) -> String {
        match self {
            RemoteReference::Oci(r) => r.locator(),
            RemoteReference::File(f) => f.locator(),
        }
    }
}

/// An `oci://` reference: registry host, repository, optional tag, optional
/// pinned OCI manifest digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciReference {
    /// `host[:port]` of the registry.
    pub registry: String,
    /// Repository path within the registry (lowercase, slash-separated).
    pub repository: String,
    /// Mutable tag, when given (`:tag`).
    pub tag: Option<String>,
    /// Pinned OCI manifest digest, when given (`@sha256:<hex>`).
    pub digest: Option<String>,
}

impl OciReference {
    fn parse_rest(input: &str, rest: &str) -> Result<OciReference> {
        let bad = |why: &str| {
            Error::usage(format!(
                "'{input}' is not a valid oci:// reference: {why} \
                 (expected oci://<registry>/<repository>[:tag][@sha256:<digest>])"
            ))
        };

        // Digest first: everything after '@' must be a full sha256 reference.
        let (rest, digest) = match rest.split_once('@') {
            Some((head, dg)) => {
                if !is_sha256_ref(dg) {
                    return Err(bad(&format!(
                        "'@{dg}' is not a sha256:<64 hex chars> digest"
                    )));
                }
                (head, Some(dg.to_string()))
            }
            None => (rest, None),
        };

        let (registry, repo_and_tag) = rest
            .split_once('/')
            .ok_or_else(|| bad("missing a /<repository> after the registry host"))?;
        if registry.is_empty() {
            return Err(bad("empty registry host"));
        }

        // The tag separator is a ':' after the last '/', so a registry port
        // (oci://host:5000/repo) never reads as a tag.
        let (repository, tag) = match repo_and_tag.rsplit_once(':') {
            Some((repo, tag)) if !repo.contains('/') || !tag.contains('/') => {
                (repo, Some(tag.to_string()))
            }
            _ => (repo_and_tag, None),
        };
        if repository.is_empty() {
            return Err(bad("empty repository path"));
        }
        if !repository
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-/".contains(&b))
        {
            return Err(bad(&format!(
                "repository '{repository}' may only contain lowercase letters, digits, and ._-/"
            )));
        }
        if let Some(tag) = &tag {
            let valid = !tag.is_empty()
                && tag.len() <= 128
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
            if !valid {
                return Err(bad(&format!("'{tag}' is not a valid tag")));
            }
        }

        Ok(OciReference {
            registry: registry.to_string(),
            repository: repository.to_string(),
            tag,
            digest,
        })
    }

    /// The canonical locator: digest-pinned when a digest is known.
    pub fn locator(&self) -> String {
        let mut s = format!("oci://{}/{}", self.registry, self.repository);
        if let Some(tag) = &self.tag {
            s.push(':');
            s.push_str(tag);
        }
        if let Some(digest) = &self.digest {
            s.push('@');
            s.push_str(digest);
        }
        s
    }

    /// The manifest reference to ask the registry for: the digest when pinned,
    /// otherwise the tag.
    pub fn manifest_reference(&self) -> Result<&str> {
        if let Some(digest) = &self.digest {
            return Ok(digest);
        }
        if let Some(tag) = &self.tag {
            return Ok(tag);
        }
        Err(Error::usage(format!(
            "'{}' names neither a tag nor a digest",
            self.locator()
        )))
    }
}

/// A `file://` reference to a producer output directory (dist dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReference {
    /// The referenced directory, as given (platform-native path).
    pub path: camino::Utf8PathBuf,
}

impl FileReference {
    fn parse_rest(input: &str, rest: &str) -> Result<FileReference> {
        // Accept file:///C:/x (RFC 8089 with a drive letter), file:///abs, and
        // file://relative. Percent-encoding is deliberately not interpreted —
        // OpenStrata paths are UTF-8 and the CLI passes them through verbatim.
        let mut path = rest;
        if let Some(stripped) = path.strip_prefix('/') {
            let bytes = stripped.as_bytes();
            if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                path = stripped; // file:///C:/… → C:/…
            }
        }
        if path.is_empty() {
            return Err(Error::usage(format!(
                "'{input}' has an empty file path (expected file://<dist-dir>)"
            )));
        }
        Ok(FileReference {
            path: camino::Utf8PathBuf::from(path),
        })
    }

    pub fn locator(&self) -> String {
        format!("file://{}", self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oci(input: &str) -> OciReference {
        match RemoteReference::parse(input).unwrap() {
            RemoteReference::Oci(r) => r,
            other => panic!("expected an OCI reference, got {other:?}"),
        }
    }

    #[test]
    fn parses_tagged_reference() {
        let r = oci("oci://ghcr.io/owner/openstrata-runtime:usd-24.08-linux-x86_64");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "owner/openstrata-runtime");
        assert_eq!(r.tag.as_deref(), Some("usd-24.08-linux-x86_64"));
        assert!(r.digest.is_none());
        assert!(!RemoteReference::Oci(r).is_pinned());
    }

    #[test]
    fn parses_digest_pinned_reference() {
        let dg = format!("sha256:{}", "ab".repeat(32));
        let r = oci(&format!("oci://ghcr.io/owner/rt@{dg}"));
        assert_eq!(r.digest.as_deref(), Some(dg.as_str()));
        assert!(r.tag.is_none());
        assert_eq!(r.manifest_reference().unwrap(), dg);
        assert!(RemoteReference::Oci(r).is_pinned());
    }

    #[test]
    fn parses_tag_and_digest_together() {
        let dg = format!("sha256:{}", "cd".repeat(32));
        let r = oci(&format!("oci://ghcr.io/owner/rt:v1@{dg}"));
        assert_eq!(r.tag.as_deref(), Some("v1"));
        // The digest is the contract; the manifest is requested by digest.
        assert_eq!(r.manifest_reference().unwrap(), dg);
    }

    #[test]
    fn registry_port_is_not_a_tag() {
        let r = oci("oci://localhost:5000/fixtures/rt");
        assert_eq!(r.registry, "localhost:5000");
        assert_eq!(r.repository, "fixtures/rt");
        assert!(r.tag.is_none());

        let r = oci("oci://localhost:5000/fixtures/rt:v2");
        assert_eq!(r.registry, "localhost:5000");
        assert_eq!(r.tag.as_deref(), Some("v2"));
    }

    #[test]
    fn malformed_references_are_usage_errors() {
        for input in [
            "ghcr.io/owner/rt",                    // no scheme
            "oci://ghcr.io",                       // no repository
            "oci:///owner/rt",                     // empty host
            "oci://ghcr.io/Owner/rt",              // uppercase repository
            "oci://ghcr.io/owner/rt@sha256:short", // malformed digest
            "oci://ghcr.io/owner/rt@md5:abc",      // wrong algorithm
            "oci://ghcr.io/owner/rt:",             // empty tag
            "oci://ghcr.io/owner/rt:bad tag",      // tag with a space
            "file://",                             // empty path
            "https://ghcr.io/owner/rt",            // unsupported scheme
        ] {
            let err = RemoteReference::parse(input).expect_err(input);
            assert_eq!(err.code(), "INVALID_ARGUMENT", "input: {input}");
        }
    }

    #[test]
    fn file_reference_forms() {
        match RemoteReference::parse("file:///C:/dist/out").unwrap() {
            RemoteReference::File(f) => assert_eq!(f.path.as_str(), "C:/dist/out"),
            other => panic!("{other:?}"),
        }
        match RemoteReference::parse("file:///opt/dist").unwrap() {
            RemoteReference::File(f) => assert_eq!(f.path.as_str(), "/opt/dist"),
            other => panic!("{other:?}"),
        }
        assert!(RemoteReference::parse("file:///opt/dist")
            .unwrap()
            .is_pinned());
    }

    #[test]
    fn locator_roundtrip() {
        let dg = format!("sha256:{}", "ef".repeat(32));
        for input in [
            "oci://ghcr.io/owner/rt:v1".to_string(),
            format!("oci://ghcr.io/owner/rt@{dg}"),
            format!("oci://ghcr.io/owner/rt:v1@{dg}"),
            "file:///opt/dist".to_string(),
        ] {
            assert_eq!(RemoteReference::parse(&input).unwrap().locator(), input);
        }
    }
}
