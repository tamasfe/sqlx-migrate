use std::borrow::Cow;

use thiserror::Error;

use crate::MigrationError;

/// An aggregated error type for the [`Migrator`].
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Database(sqlx::Error),
    #[error(
        "invalid version specified: {version} (available versions: {min_version}-{max_version})"
    )]
    InvalidVersion {
        version: u64,
        min_version: u64,
        max_version: u64,
    },
    #[error("there were no local migrations found")]
    NoMigrations,
    #[error("missing migrations ({local_count} local, but {db_count} already applied)")]
    MissingMigrations { local_count: usize, db_count: usize },
    #[error("error applying migration: {error}")]
    Migration {
        name: Cow<'static, str>,
        version: u64,
        error: MigrationError,
    },
    #[error("error reverting migration: {error}")]
    Revert {
        name: Cow<'static, str>,
        version: u64,
        error: MigrationError,
    },
    #[error("expected migration {version} to be {local_name} but it was applied as {db_name}")]
    NameMismatch {
        version: u64,
        local_name: Cow<'static, str>,
        db_name: Cow<'static, str>,
    },
    #[error("invalid checksum for migration {version}")]
    ChecksumMismatch {
        version: u64,
        local_checksum: Cow<'static, [u8]>,
        db_checksum: Cow<'static, [u8]>,
    },
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}
