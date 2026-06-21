//! Error type shared across OpenStrata crates.
//!
//! Per the quality bar ("CLI errors must be actionable"), variants carry the
//! offending identifier so the CLI layer can render a useful message.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("platform '{0}' not found (try `ost platform list`)")]
    PlatformNotFound(String),

    #[error("manifest is invalid: {0}")]
    InvalidManifest(String),

    /// An operational failure with a self-contained, actionable message.
    #[error("{0}")]
    Operation(String),

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
}
