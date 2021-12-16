//! # SQLx Migrate
//! 
//! An opinionated migration micro-framework that uses [SQLx](https://github.com/launchbadge/sqlx).
//! 
//! All migrations are written in Rust, and it is designed to embedded in existing applications.
//! 

#![deny(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::unreadable_literal,
    clippy::doc_markdown
)]

use db::{AppliedMigration, Migrations};
use futures_core::future::LocalBoxFuture;
use itertools::{EitherOrBoth, Itertools};
use sqlx::{ConnectOptions, Connection, Database, Pool, Transaction};
use std::{
    borrow::Cow,
    str::FromStr,
    time::{Duration, Instant},
};
use thiserror::Error;

pub mod db;

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "generate")]
mod gen;

#[cfg(feature = "generate")]
pub use gen::generate;

type MigrationFn<DB> = Box<
    dyn for<'future> Fn(
        &'future mut Transaction<DB>,
    ) -> LocalBoxFuture<'future, Result<(), MigrationError>>,
>;

/// The default migrations table used by all migrators.
pub const DEFAULT_MIGRATIONS_TABLE: &str = "_sqlx_migrations";

pub mod prelude {
    pub use super::Error;
    pub use super::Migration;
    pub use super::MigrationError;
    pub use super::MigrationStatus;
    pub use super::MigrationSummary;
    pub use super::Migrator;
    pub use super::MigratorOptions;
}

/// A single migration that uses a given [`sqlx::Transaction`] to do the up (migrate) and down (revert) migrations.
///
/// # Example
///
/// ```
/// use sqlx_migrate::Migration;
/// use sqlx::{Executor, Postgres};
///
/// let migration = Migration::<Postgres>::new("initial migration", |tx| {
///     Box::pin(async move {
///         tx.execute("CREATE TABLE example ();").await?;
///         Ok(())
///     })
/// })
/// // Low-effort (optional) checksum.
/// .with_checksum(b"CREATE TABLE example ();".as_slice())
/// .reversible(|tx| {
///     Box::pin(async move {
///         tx.execute("DROP TABLE example;");
///         Ok(())
///     })
/// });
/// ```
pub struct Migration<DB: Database> {
    name: Cow<'static, str>,
    checksum: Cow<'static, [u8]>,
    up: MigrationFn<DB>,
    down: Option<MigrationFn<DB>>,
}

impl<DB: Database> Migration<DB> {
    /// Create a new migration with the given name
    /// and migration function.
    pub fn new(
        name: impl Into<Cow<'static, str>>,
        up: impl for<'future> Fn(
                &'future mut Transaction<DB>,
            ) -> LocalBoxFuture<'future, Result<(), MigrationError>>
            + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            checksum: Cow::default(),
            up: Box::new(up),
            down: None,
        }
    }

    /// Set a down migration function.
    pub fn reversible(
        mut self,
        down: impl for<'future> Fn(
                &'future mut Transaction<DB>,
            ) -> LocalBoxFuture<'future, Result<(), MigrationError>>
            + 'static,
    ) -> Self {
        self.down = Some(Box::new(down));
        self
    }

    /// Set a checksum for the migration.
    ///
    /// A checksum is only useful for migrations that come from external sources.
    pub fn with_checksum(mut self, checksum: impl Into<Cow<'static, [u8]>>) -> Self {
        self.checksum = checksum.into();
        self
    }

    /// Get the migration's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Get a reference to the migration's checksum.
    #[must_use]
    pub fn checksum(&self) -> &[u8] {
        self.checksum.as_ref()
    }

    /// Whether the migration is reversible or not.
    #[must_use]
    pub fn is_reversible(&self) -> bool {
        self.down.is_some()
    }
}

impl<DB: Database> Eq for Migration<DB> {}
impl<DB: Database> PartialEq for Migration<DB> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.checksum == other.checksum
    }
}

/// A Migrator that is capable of managing migrations for a database.
///
/// # Example
///
/// ```no_run
/// use crate::{Error, Migration, Migrator};
/// use sqlx::{Executor, Postgres};
///
/// async fn migrate() -> Result<(), Error> {
///     let mut migrator: Migrator<Postgres> =
///         Migrator::connect("postgres://postgres:postgres@localhost:5432/postgres").await?;
///
///     let migration = Migration::<Postgres>::new("initial migration", |tx| {
///         Box::pin(async move {
///             tx.execute("CREATE TABLE example ();").await?;
///             Ok(())
///         })
///     })
///     .with_checksum(b"CREATE TABLE example ();".as_slice())
///     .reversible(|tx| {
///         Box::pin(async move {
///             tx.execute("DROP TABLE example;");
///             Ok(())
///         })
///     });
///
///     migrator.add_migrations([migration]);
///
///     // Make sure all migrations are consistent with the database.
///     migrator.check_migrations().await?;
///
///     // Migrate
///     let summary = migrator.migrate(migrator.local_migrations().len() as _).await?;
///
///     assert_eq!(summary.new_version, Some(1));
///
///     // List all migrations.
///     let status = migrator.status().await?;
///
///     // Verify that all of them are applied.
///     for migration in status {
///         assert!(migration.applied.is_some());
///     }
///
///     Ok(())
/// }
/// ```
pub struct Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    options: MigratorOptions,
    conn: DB::Connection,
    table: Cow<'static, str>,
    migrations: Vec<Migration<DB>>,
}

impl<DB> Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    /// Create a new migrator that uses an existing connection.
    pub fn new(conn: DB::Connection) -> Self {
        Self {
            options: MigratorOptions::default(),
            conn,
            table: Cow::Borrowed(DEFAULT_MIGRATIONS_TABLE),
            migrations: Vec::default(),
        }
    }

    /// Connect to a database given in the URL.
    /// 
    /// If this method is used, `SQLx` statement logging is explicitly disabled.
    /// To customize the connection, use [`Migrator::connect_with`].
    /// 
    /// # Errors
    /// 
    /// An error is returned on connection failure.
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let mut opts: <<DB as Database>::Connection as Connection>::Options = url.parse()?;
        opts.disable_statement_logging();

        Ok(Self {
            options: MigratorOptions::default(),
            conn: DB::Connection::connect_with(&opts).await?,
            table: Cow::Borrowed(DEFAULT_MIGRATIONS_TABLE),
            migrations: Vec::default(),
        })
    }

    /// Connect to a database with the given connection options.
    /// 
    /// # Errors
    /// 
    /// An error is returned on connection failure.
    pub async fn connect_with(
        options: &<DB::Connection as Connection>::Options,
    ) -> Result<Self, sqlx::Error> {
        Ok(Self {
            options: MigratorOptions::default(),
            conn: DB::Connection::connect_with(options).await?,
            table: Cow::Borrowed(DEFAULT_MIGRATIONS_TABLE),
            migrations: Vec::default(),
        })
    }

    /// Use a connection from an existing connection pool.
    /// 
    /// **note**: A connection will be detached from the pool.
    /// 
    /// # Errors
    /// 
    /// An error is returned on connection failure.
    pub async fn connect_with_pool(pool: &Pool<DB>) -> Result<Self, sqlx::Error> {
        let conn = pool.acquire().await?;

        Ok(Self {
            options: MigratorOptions::default(),
            conn: conn.detach(),
            table: Cow::Borrowed(DEFAULT_MIGRATIONS_TABLE),
            migrations: Vec::default(),
        })
    }

    /// Set the table name for migration bookkeeping to override the default [`DEFAULT_MIGRATIONS_TABLE`].
    ///
    /// The table name is used as-is in queries, **DO NOT USE UNTRUSTED STRINGS**.
    pub fn set_migrations_table(&mut self, name: impl AsRef<str>) {
        self.table = Cow::Owned(name.as_ref().to_string());
    }

    /// Add migrations to the migrator.
    pub fn add_migrations(&mut self, migrations: impl IntoIterator<Item = Migration<DB>>) {
        self.migrations.extend(migrations.into_iter());
    }

    /// Override the migrator's options.
    pub fn set_options(&mut self, options: MigratorOptions) {
        self.options = options;
    }

    /// List all local migrations.
    /// 
    /// To list all migrations, use [`Self::status`].
    pub fn local_migrations(&self) -> &[Migration<DB>] {
        &self.migrations
    }
}

impl<DB> Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    /// Apply all migrations to the given version.
    /// 
    /// Migration versions start at 1 and migrations are ordered
    /// the way they were added to the migrator.
    /// 
    /// # Errors
    /// 
    /// Whenever a migration fails, and error is returned and no database
    /// changes will be made.
    pub async fn migrate(&mut self, version: u64) -> Result<MigrationSummary, Error> {
        self.local_migration(version)?;

        self.check_migrations().await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        let to_apply = self
            .migrations
            .iter()
            .enumerate()
            .skip_while(|(idx, _)| *idx < db_migrations.len())
            .take_while(|(idx, _)| *idx < version as _);

        let mut tx = self.conn.begin().await?;

        let version = version.max(db_migrations.len() as _);

        for (idx, mig) in to_apply {
            let version = idx as u64 + 1;

            let start = Instant::now();

            tracing::info!(
                version,
                name = %mig.name,
                "applying migration"
            );

            (&*mig.up)(&mut tx)
                .await
                .map_err(|error| Error::Migration {
                    name: mig.name.clone(),
                    version,
                    error,
                })?;

            let execution_time = Instant::now() - start;

            DB::Connection::add_migration(
                &self.table,
                AppliedMigration {
                    version,
                    name: mig.name.clone(),
                    checksum: mig.checksum.clone(),
                    execution_time,
                },
                &mut tx,
            )
            .await?;

            tracing::info!(
                version,
                name = %mig.name,
                execution_time = %humantime::Duration::from(execution_time),
                "migration applied"
            );
        }

        tracing::info!("committing changes");
        tx.commit().await?;

        Ok(MigrationSummary {
            old_version: if db_migrations.is_empty() {
                None
            } else {
                Some(db_migrations.len() as _)
            },
            new_version: Some(version),
        })
    }

    /// Apply all local migrations, if there are any.
    /// 
    /// # Errors
    /// 
    /// Uses [`Self::migrate`] internally, errors are propagated.
    pub async fn migrate_all(&mut self) -> Result<MigrationSummary, Error> {
        self.check_migrations().await?;

        if self.migrations.is_empty() {
            return Ok(MigrationSummary {
                new_version: None,
                old_version: None,
            });
        }

        self.migrate(self.migrations.len() as _).await
    }

    /// Revert all migrations after and including the given version.
    /// 
    /// Any migrations that are "not reversible" and have no revert functions will be ignored.
    /// 
    /// # Errors
    /// 
    /// Whenever a migration fails, and error is returned and no database
    /// changes will be made.    
    pub async fn revert(&mut self, version: u64) -> Result<MigrationSummary, Error> {
        self.local_migration(version)?;

        self.check_migrations().await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        let to_revert = self
            .migrations
            .iter()
            .enumerate()
            .skip_while(|(idx, _)| idx + 1 < version as _)
            .take_while(|(idx, _)| *idx < db_migrations.len())
            .collect::<Vec<_>>()
            .into_iter()
            .rev();

        let mut tx = self.conn.begin().await?;

        for (idx, mig) in to_revert {
            let version = idx as u64 + 1;

            let start = Instant::now();

            tracing::info!(
                version,
                name = %mig.name,
                "reverting migration"
            );

            match &mig.down {
                Some(down) => {
                    down(&mut tx).await.map_err(|error| Error::Revert {
                        name: mig.name.clone(),
                        version,
                        error,
                    })?;
                }
                None => {
                    tracing::warn!(
                        version,
                        name = %mig.name,
                        "no down migration found"
                    );
                }
            }

            let execution_time = Instant::now() - start;

            DB::Connection::remove_migration(&self.table, version, &mut tx).await?;

            tracing::info!(
                version,
                name = %mig.name,
                execution_time = %humantime::Duration::from(execution_time),
                "migration reverted"
            );
        }

        tracing::info!("committing changes");
        tx.commit().await?;

        Ok(MigrationSummary {
            old_version: if db_migrations.is_empty() {
                None
            } else {
                Some(db_migrations.len() as _)
            },
            new_version: if version == 1 {
                None
            } else {
                Some(version - 1)
            },
        })
    }

    /// Revert all applied migrations, if any.
    /// 
    /// # Errors
    /// 
    /// Uses [`Self::revert`], any errors will be propagated.
    pub async fn revert_all(&mut self) -> Result<MigrationSummary, Error> {
        self.check_migrations().await?;

        if self.migrations.is_empty() {
            return Ok(MigrationSummary {
                new_version: None,
                old_version: None,
            });
        }

        self.revert(1).await
    }

    /// Force 
    /// 
    /// # Errors
    /// 
    /// The forced migration version must exist locally.
    /// 
    /// Connection and database errors are returned.
    /// 
    /// Truncating the migrations table and applying migrations are done
    /// in separate transactions. As a consequence in some occasions
    /// the migrations table might be cleared and no migrations will be set.
    /// 
    /// This function should be considered (almost) idempotent, and repeatedly calling it
    /// should result in the same state. Some database-specific values can change, such as timestamps.
    pub async fn force_version(&mut self, version: u64) -> Result<MigrationSummary, Error> {
        self.local_migration(version)?;

        self.conn.ensure_migrations_table(&self.table).await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        let migrations = self
            .migrations
            .iter()
            .enumerate()
            .take_while(|(idx, _)| *idx < version as usize);

        self.conn.clear_migrations(&self.table).await?;

        let mut tx = self.conn.begin().await?;

        for (idx, mig) in migrations {
            DB::Connection::add_migration(
                &self.table,
                AppliedMigration {
                    version: idx as u64 + 1,
                    name: mig.name.clone(),
                    checksum: mig.checksum.clone(),
                    execution_time: Duration::default(),
                },
                &mut tx,
            )
            .await?;

            tracing::info!(
                version = idx + 1,
                name = %mig.name,
                "migration forcibly set as applied"
            );
        }

        tracing::info!("committing changes");
        tx.commit().await?;

        Ok(MigrationSummary {
            old_version: if db_migrations.is_empty() {
                None
            } else {
                Some(db_migrations.len() as _)
            },
            new_version: Some(version),
        })
    }

    /// Verify all the migrations.
    /// 
    /// # Errors
    /// 
    /// The following kind of errors can be returned:
    /// 
    /// - connection and database errors
    /// - mismatch errors
    /// 
    /// Mismatch errors can happen if the local migrations'
    /// name or checksum does not match the applied migration's.
    /// 
    /// Both name and checksum validation can be turned off via [`MigratorOptions`].
    pub async fn verify(&mut self) -> Result<(), Error> {
        self.check_migrations().await
    }

    /// List all local and applied migrations.
    /// 
    /// # Errors
    /// 
    /// Errors are returned on connection and database errors.
    /// The migrations themselves are not verified.
    pub async fn status(&mut self) -> Result<Vec<MigrationStatus>, Error> {
        self.conn.ensure_migrations_table(&self.table).await?;

        let migrations = self.conn.list_migrations(&self.table).await?;

        let mut status = Vec::with_capacity(self.migrations.len());

        for (idx, pair) in self
            .migrations
            .iter()
            .zip_longest(migrations.into_iter())
            .enumerate()
        {
            let version = idx as u64 + 1;

            match pair {
                EitherOrBoth::Both(local, db) => status.push(MigrationStatus {
                    version,
                    name: local.name.clone().into_owned(),
                    reversible: local.is_reversible(),
                    checksum: local.checksum.clone().into_owned(),
                    applied: Some(db),
                    missing_local: false,
                }),
                EitherOrBoth::Left(local) => status.push(MigrationStatus {
                    version,
                    name: local.name.clone().into_owned(),
                    reversible: local.is_reversible(),
                    checksum: local.checksum.clone().into_owned(),
                    applied: None,
                    missing_local: false,
                }),
                EitherOrBoth::Right(r) => status.push(MigrationStatus {
                    version: r.version,
                    name: r.name.clone().into_owned(),
                    checksum: Vec::default(),
                    reversible: false,
                    applied: Some(r),
                    missing_local: true,
                }),
            }
        }

        Ok(status)
    }
}

impl<DB> Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    fn local_migration(&self, version: u64) -> Result<&Migration<DB>, Error> {
        if version == 0 {
            return Err(Error::InvalidVersion {
                version,
                min_version: 1,
                max_version: self.migrations.len() as _,
            });
        }

        if self.migrations.is_empty() {
            return Err(Error::InvalidVersion {
                version,
                min_version: 1,
                max_version: self.migrations.len() as _,
            });
        }

        let idx = version - 1;

        self.migrations
            .get(idx as usize)
            .ok_or(Error::InvalidVersion {
                version,
                min_version: 1,
                max_version: self.migrations.len() as _,
            })
    }

    async fn check_migrations(&mut self) -> Result<(), Error> {
        self.conn.ensure_migrations_table(&self.table).await?;

        let migrations = self.conn.list_migrations(&self.table).await?;

        if self.migrations.len() < migrations.len() {
            return Err(Error::MissingMigrations {
                local_count: self.migrations.len(),
                db_count: migrations.len(),
            });
        }

        for (idx, (db_migration, local_migration)) in migrations
            .into_iter()
            .zip(self.migrations.iter())
            .enumerate()
        {
            let version = idx as u64 + 1;

            if self.options.verify_names && db_migration.name != local_migration.name {
                return Err(Error::NameMismatch {
                    version,
                    local_name: local_migration.name.clone(),
                    db_name: db_migration.name.clone(),
                });
            }

            if self.options.verify_checksums && db_migration.checksum != local_migration.checksum {
                return Err(Error::ChecksumMismatch {
                    version,
                    local_checksum: local_migration.checksum.clone(),
                    db_checksum: db_migration.checksum.clone(),
                });
            }
        }

        Ok(())
    }
}

/// Options for a [`Migrator`].
#[derive(Debug)]
pub struct MigratorOptions {
    /// Whether to check applied migration checksums.
    pub verify_checksums: bool,
    /// Whether to check applied migration names.
    pub verify_names: bool,
}

impl Default for MigratorOptions {
    fn default() -> Self {
        Self {
            verify_checksums: true,
            verify_names: true,
        }
    }
}

/// Summary of a migration or revert operation.
#[derive(Debug, Clone)]
pub struct MigrationSummary {
    /// The old migration version in the database.
    pub old_version: Option<u64>,
    /// The new migration version in the database.
    pub new_version: Option<u64>,
}

/// Status of a migration.
#[derive(Debug, Clone)]
pub struct MigrationStatus {
    /// Migration version determined by migration order.
    pub version: u64,
    /// The name of the migration.
    pub name: String,
    /// Whether the migration has a reverse function.
    pub reversible: bool,
    /// Migration checksum, if any.
    pub checksum: Vec<u8>,
    /// Information about the migration in the database.
    pub applied: Option<db::AppliedMigration<'static>>,
    /// Whether the migration is found in the database,
    /// but missing locally.
    pub missing_local: bool,
}

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

/// An opaque error type returned by user-provided migration functions.
/// 
/// Currently [`anyhow::Error`] is used, but it should be considered an implementation detail.
pub type MigrationError = anyhow::Error;

/// An `SQLx` database type, used for code generation purposes.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum DatabaseType {
    Postgres,
    Any,
}

impl DatabaseType {
    fn sqlx_type(self) -> &'static str {
        match self {
            DatabaseType::Postgres => "Postgres",
            DatabaseType::Any => "Any",
        }
    }
}

impl FromStr for DatabaseType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "postgres" => Ok(Self::Postgres),
            "any" => Ok(Self::Any),
            db => Err(anyhow::anyhow!("invalid database type `{}`", db)),
        }
    }
}
