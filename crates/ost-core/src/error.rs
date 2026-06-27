// SPDX-License-Identifier: Apache-2.0
//! Error type shared across OpenStrata crates.
//!
//! Per the quality bar ("CLI errors must be actionable"), variants carry the
//! offending identifier so the CLI layer can render a useful message.
//!
//! Beyond a message, every error exposes a **stable machine code** and a
//! **category** (design §14.4) so agents and CI can branch on cause without
//! matching prose. The category determines the process exit code; the raw
//! string `code` is surfaced in `--json` output. Codes and exit codes are part
//! of the public contract — extend them additively, never repurpose them.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Cause category for an [`Error`] (design §14.4). Maps one-to-one onto the
/// process exit code, so CI can branch on the *kind* of failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Bad arguments or usage. Exit `2`.
    Usage,
    /// Invalid manifest, lock, or configuration. Exit `3`.
    Configuration,
    /// A missing prerequisite: runtime, tool, directory. Exit `4`.
    Precondition,
    /// A validation mismatch (lock check, validate, plugin test). Exit `5`.
    Validation,
    /// An external tool failed (CMake, Ninja, compiler, OpenUSD). Exit `6`.
    ExternalTool,
    /// Filesystem or permission error. Exit `7`.
    Io,
    /// An unexpected internal error. Exit `70`.
    Internal,
}

impl Category {
    /// The normalized process exit code for this category (design §14.4).
    pub fn exit_code(self) -> u8 {
        match self {
            Category::Usage => 2,
            Category::Configuration => 3,
            Category::Precondition => 4,
            Category::Validation => 5,
            Category::ExternalTool => 6,
            Category::Io => 7,
            Category::Internal => 70,
        }
    }

    /// The stable lowercase tag used in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Usage => "usage",
            Category::Configuration => "configuration",
            Category::Precondition => "precondition",
            Category::Validation => "validation",
            Category::ExternalTool => "external_tool",
            Category::Io => "io",
            Category::Internal => "internal",
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("platform '{0}' not found (try `ost platform list`)")]
    PlatformNotFound(String),

    #[error("manifest is invalid: {0}")]
    InvalidManifest(String),

    /// A legacy operational failure with a self-contained, actionable message.
    /// New code should prefer a categorized constructor ([`Error::precondition`]
    /// etc.) so the cause is machine-classifiable; this variant defaults to
    /// [`Category::Precondition`] during the migration (design §14.4).
    #[error("{0}")]
    Operation(String),

    /// A categorized error carrying a stable machine `code`, a [`Category`], and
    /// an optional actionable `hint` (design §14.3/§14.4).
    #[error("{message}")]
    Coded {
        code: &'static str,
        category: Category,
        message: String,
        hint: Option<String>,
    },

    #[error("a project already exists here: {0}")]
    ProjectExists(String),

    #[error("no OpenStrata project found in '{0}' or any parent (run `ost init`)")]
    ProjectNotFound(String),

    #[error("i/o error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse {what}: {source}")]
    Parse {
        what: String,
        #[source]
        source: anyhow::Error,
    },
}

impl Error {
    /// Attach a filesystem path to a raw [`std::io::Error`].
    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Wrap a parse failure with a human label for the thing being parsed.
    pub fn parse(what: impl Into<String>, source: impl Into<anyhow::Error>) -> Self {
        Error::Parse {
            what: what.into(),
            source: source.into(),
        }
    }

    /// A categorized error with an explicit stable code (design §14.4).
    pub fn coded(code: &'static str, category: Category, message: impl Into<String>) -> Self {
        Error::Coded {
            code,
            category,
            message: message.into(),
            hint: None,
        }
    }

    /// A bad-argument / usage error (`INVALID_ARGUMENT`, exit `2`).
    pub fn usage(message: impl Into<String>) -> Self {
        Error::coded("INVALID_ARGUMENT", Category::Usage, message)
    }

    /// An invalid-configuration error (`INVALID_CONFIG`, exit `3`).
    pub fn config(message: impl Into<String>) -> Self {
        Error::coded("INVALID_CONFIG", Category::Configuration, message)
    }

    /// A missing-prerequisite error (`PRECONDITION_FAILED`, exit `4`).
    pub fn precondition(message: impl Into<String>) -> Self {
        Error::coded("PRECONDITION_FAILED", Category::Precondition, message)
    }

    /// A validation-mismatch error (`VALIDATION_FAILED`, exit `5`).
    pub fn validation(message: impl Into<String>) -> Self {
        Error::coded("VALIDATION_FAILED", Category::Validation, message)
    }

    /// An external-tool failure (`EXTERNAL_TOOL_FAILED`, exit `6`).
    pub fn external_tool(message: impl Into<String>) -> Self {
        Error::coded("EXTERNAL_TOOL_FAILED", Category::ExternalTool, message)
    }

    /// Attach an actionable hint, kept separate from the message for `--json`.
    /// A no-op on variants that do not carry a hint slot.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        if let Error::Coded { hint: slot, .. } = &mut self {
            *slot = Some(hint.into());
        }
        self
    }

    /// The stable machine code for this error (design §14.4).
    pub fn code(&self) -> &'static str {
        match self {
            Error::PlatformNotFound(_) => "PLATFORM_NOT_FOUND",
            Error::InvalidManifest(_) => "MANIFEST_INVALID",
            Error::ProjectExists(_) => "PROJECT_EXISTS",
            Error::ProjectNotFound(_) => "PROJECT_NOT_FOUND",
            Error::Io { .. } => "IO_ERROR",
            Error::Parse { .. } => "PARSE_FAILED",
            Error::Coded { code, .. } => code,
            // Legacy generic failures default to a precondition (see variant).
            Error::Operation(_) => "OPERATION_FAILED",
        }
    }

    /// The cause category for this error (design §14.4).
    pub fn category(&self) -> Category {
        match self {
            // A platform id comes from CLI input or the manifest; a bad one is a
            // usage error the caller can correct.
            Error::PlatformNotFound(_) => Category::Usage,
            Error::InvalidManifest(_) => Category::Configuration,
            Error::Parse { .. } => Category::Configuration,
            Error::ProjectExists(_) => Category::Usage,
            Error::ProjectNotFound(_) => Category::Precondition,
            Error::Io { .. } => Category::Io,
            Error::Coded { category, .. } => *category,
            // Most legacy operational failures are missing prerequisites; the
            // remaining usage/validation/external cases are migrated to
            // categorized constructors crate by crate.
            Error::Operation(_) => Category::Precondition,
        }
    }

    /// The normalized process exit code for this error (design §14.4).
    pub fn exit_code(&self) -> u8 {
        self.category().exit_code()
    }

    /// The actionable hint, if any (carried only by [`Error::Coded`]).
    pub fn hint(&self) -> Option<&str> {
        match self {
            Error::Coded { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_map_to_design_exit_codes() {
        assert_eq!(Category::Usage.exit_code(), 2);
        assert_eq!(Category::Configuration.exit_code(), 3);
        assert_eq!(Category::Precondition.exit_code(), 4);
        assert_eq!(Category::Validation.exit_code(), 5);
        assert_eq!(Category::ExternalTool.exit_code(), 6);
        assert_eq!(Category::Io.exit_code(), 7);
        assert_eq!(Category::Internal.exit_code(), 70);
    }

    #[test]
    fn typed_variants_carry_stable_codes_and_categories() {
        let e = Error::PlatformNotFound("cy2099".into());
        assert_eq!(e.code(), "PLATFORM_NOT_FOUND");
        assert_eq!(e.category(), Category::Usage);
        assert_eq!(e.exit_code(), 2);

        let e = Error::InvalidManifest("bad".into());
        assert_eq!(e.code(), "MANIFEST_INVALID");
        assert_eq!(e.exit_code(), 3);
    }

    #[test]
    fn legacy_operation_defaults_to_precondition() {
        let e = Error::Operation("runtime not pulled".into());
        assert_eq!(e.code(), "OPERATION_FAILED");
        assert_eq!(e.category(), Category::Precondition);
        assert_eq!(e.exit_code(), 4);
        assert!(e.hint().is_none());
    }

    #[test]
    fn coded_constructors_set_code_category_and_hint() {
        let e = Error::usage("unknown plugin kind 'foo'");
        assert_eq!(e.code(), "INVALID_ARGUMENT");
        assert_eq!(e.exit_code(), 2);

        let e = Error::external_tool("cmake configure failed (exit 1)")
            .with_hint("see build.log for details");
        assert_eq!(e.code(), "EXTERNAL_TOOL_FAILED");
        assert_eq!(e.category(), Category::ExternalTool);
        assert_eq!(e.hint(), Some("see build.log for details"));

        let e = Error::coded("REAL_RUNTIME_REQUIRED", Category::Precondition, "need real usd");
        assert_eq!(e.code(), "REAL_RUNTIME_REQUIRED");
        assert_eq!(e.exit_code(), 4);
    }
}
