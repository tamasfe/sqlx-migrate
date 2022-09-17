//! # SQLx Migrate
//!
//! An opinionated migration micro-framework that uses [SQLx](https://github.com/launchbadge/sqlx).
//!
//! All migrations are written in Rust, and are designed to be embedded in existing applications.
//!
#![cfg_attr(feature = "_docs", feature(doc_cfg))]
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::unreadable_literal,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

use context::Extensions;
use db::{AppliedMigration, Migrations};
use futures_core::future::LocalBoxFuture;
use itertools::{EitherOrBoth, Itertools};
use sha2::{Digest, Sha256};
use sqlx::{ConnectOptions, Connection, Database, Pool};
use std::{
    any::TypeId,
    borrow::Cow,
    cell::UnsafeCell,
    str::FromStr,
    time::{Duration, Instant},
};

pub mod context;
pub mod db;
pub mod error;

pub use context::MigrationContext;
pub use error::Error;

#[cfg(feature = "cli")]
#[cfg_attr(feature = "_docs", doc(cfg(feature = "cli")))]
pub mod cli;

#[cfg(feature = "generate")]
#[cfg_attr(feature = "_docs", doc(cfg(feature = "generate")))]
mod gen;

#[cfg(feature = "generate")]
#[cfg_attr(feature = "_docs", doc(cfg(feature = "generate")))]
pub use gen::generate;

type MigrationFn<DB> = Box<
    dyn for<'future> Fn(
        MigrationContext<'future, DB>,
    ) -> LocalBoxFuture<'future, Result<(), MigrationError>>,
>;

/// The default migrations table used by all migrators.
pub const DEFAULT_MIGRATIONS_TABLE: &str = "_sqlx_migrations";

/// Commonly used types and functions.
pub mod prelude {
    pub use super::Migration;
    pub use super::MigrationContext;
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
    up: MigrationFn<DB>,
    down: Option<MigrationFn<DB>>,
}

impl<DB: Database> Migration<DB> {
    /// Create a new migration with the given name
    /// and migration function.
    pub fn new(
        name: impl Into<Cow<'static, str>>,
        up: impl for<'future> Fn(
                MigrationContext<'future, DB>,
            ) -> LocalBoxFuture<'future, Result<(), MigrationError>>
            + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            up: Box::new(up),
            down: None,
        }
    }

    /// Set a down migration function.
    #[must_use]
    pub fn reversible(
        mut self,
        down: impl for<'future> Fn(
                MigrationContext<'future, DB>,
            ) -> LocalBoxFuture<'future, Result<(), MigrationError>>
            + 'static,
    ) -> Self {
        self.down = Some(Box::new(down));
        self
    }

    /// Same as [`Migration::reversible`]
    #[must_use]
    pub fn revertible(
        self,
        down: impl for<'future> Fn(
                MigrationContext<'future, DB>,
            ) -> LocalBoxFuture<'future, Result<(), MigrationError>>
            + 'static,
    ) -> Self {
        self.reversible(down)
    }

    /// Get the migration's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Whether the migration is reversible or not.
    #[must_use]
    pub fn is_reversible(&self) -> bool {
        self.down.is_some()
    }

    /// Whether the migration is reversible or not.
    #[must_use]
    pub fn is_revertible(&self) -> bool {
        self.down.is_some()
    }
}

impl<DB: Database> Eq for Migration<DB> {}
impl<DB: Database> PartialEq for Migration<DB> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
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
///    let mut migrator: Migrator<Postgres> =
///         Migrator::connect("postgres://postgres:postgres@localhost:5432/postgres").await?;
///
///     let migration = Migration::<Postgres>::new("initial migration", |tx| {
///         Box::pin(async move {
///             tx.execute("CREATE TABLE example ();").await?;
///             Ok(())
///         })
///     })
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
#[must_use]
pub struct Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    options: MigratorOptions,
    conn: DB::Connection,
    table: Cow<'static, str>,
    migrations: Vec<Migration<DB>>,
    extensions: UnsafeCell<Extensions>,
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
            extensions: UnsafeCell::new(Extensions::default()),
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
            extensions: UnsafeCell::new(Extensions::default()),
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
            extensions: UnsafeCell::new(Extensions::default()),
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
            extensions: UnsafeCell::new(Extensions::default()),
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

    /// With an extension that is available to the migrations.
    pub fn with<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.set(value);
        self
    }

    /// Add an extension that is available to the migrations.
    pub fn set<T: Send + Sync + 'static>(&mut self, value: T) {
        // SAFETY: Self is mutably borrowed.
        unsafe {
            (*self.extensions.get())
                .map
                .insert(TypeId::of::<T>(), Box::new(value));
        }
    }

    /// List all local migrations.
    ///
    /// To list all migrations, use [`Migrator::status`].
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
    pub async fn migrate(&mut self, target_version: u64) -> Result<MigrationSummary, Error> {
        self.local_migration(target_version)?;
        self.conn.ensure_migrations_table(&self.table).await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        self.check_migrations(&db_migrations)?;

        let to_apply = self.migrations.iter();

        let tx = UnsafeCell::new(self.conn.begin().await?);

        let db_version = db_migrations.len() as _;

        for (idx, mig) in to_apply.enumerate() {
            let mig_version = idx as u64 + 1;

            if mig_version > target_version {
                break;
            }

            if mig_version <= db_version {
                continue;
            }

            let start = Instant::now();

            tracing::info!(
                version = mig_version,
                name = %mig.name,
                "applying migration"
            );

            let hasher = UnsafeCell::new(Sha256::new());

            // First we execute the migration with dummy queries,
            // otherwise the checksum will depend on the data
            // inside the database.
            //
            // This way we miss out on queries that depend on
            // the database context.
            // FIXME: detect this and warn the user.
            let ctx = MigrationContext {
                hash_only: true,
                ext: self.extensions.get(),
                hasher: hasher.get(),
                tx: tx.get(),
            };

            (*mig.up)(ctx).await.map_err(|error| Error::Migration {
                name: mig.name.clone(),
                version: mig_version,
                error,
            })?;

            let checksum = hasher.into_inner().finalize().to_vec();

            let hasher = UnsafeCell::new(Sha256::new());

            let ctx = MigrationContext {
                hash_only: false,
                ext: self.extensions.get(),
                hasher: hasher.get(),
                tx: tx.get(),
            };

            (*mig.up)(ctx).await.map_err(|error| Error::Migration {
                name: mig.name.clone(),
                version: mig_version,
                error,
            })?;

            let execution_time = start.elapsed();

            if self.options.verify_checksums {
                if let Some(db_mig) = db_migrations.get(idx) {
                    if db_mig.checksum != checksum {
                        let tx = tx.into_inner();

                        tx.rollback().await?;

                        return Err(Error::ChecksumMismatch {
                            version: mig_version,
                            local_checksum: checksum.clone().into(),
                            db_checksum: db_mig.checksum.clone(),
                        });
                    }
                }
            }

            DB::Connection::add_migration(
                &self.table,
                AppliedMigration {
                    version: mig_version,
                    name: mig.name.clone(),
                    checksum: checksum.into(),
                    execution_time,
                },
                // SAFETY: tx was only borrowed in the context
                // that was passed into the function and no longer
                // exists.
                unsafe { &mut *tx.get() },
            )
            .await?;

            tracing::info!(
                version = mig_version,
                name = %mig.name,
                execution_time = %humantime::Duration::from(execution_time),
                "migration applied"
            );
        }

        tracing::info!("committing changes");
        tx.into_inner().commit().await?;

        Ok(MigrationSummary {
            old_version: if db_migrations.is_empty() {
                None
            } else {
                Some(db_migrations.len() as _)
            },
            new_version: Some(target_version.max(db_version)),
        })
    }

    /// Apply all local migrations, if there are any.
    ///
    /// # Errors
    ///
    /// Uses [`Migrator::migrate`] internally, errors are propagated.
    pub async fn migrate_all(&mut self) -> Result<MigrationSummary, Error> {
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
    pub async fn revert(&mut self, target_version: u64) -> Result<MigrationSummary, Error> {
        self.local_migration(target_version)?;
        self.conn.ensure_migrations_table(&self.table).await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        self.check_migrations(&db_migrations)?;

        let to_revert = self
            .migrations
            .iter()
            .enumerate()
            .skip_while(|(idx, _)| idx + 1 < target_version as _)
            .take_while(|(idx, _)| *idx < db_migrations.len())
            .collect::<Vec<_>>()
            .into_iter()
            .rev();

        let tx = UnsafeCell::new(self.conn.begin().await?);

        for (idx, mig) in to_revert {
            let version = idx as u64 + 1;

            let start = Instant::now();

            tracing::info!(
                version,
                name = %mig.name,
                "reverting migration"
            );

            let hasher = UnsafeCell::new(Sha256::new());

            let ctx = MigrationContext {
                hash_only: false,
                ext: self.extensions.get(),
                hasher: hasher.get(),
                tx: tx.get(),
            };

            match &mig.down {
                Some(down) => {
                    down(ctx).await.map_err(|error| Error::Revert {
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

            let execution_time = start.elapsed();

            DB::Connection::remove_migration(
                &self.table,
                version,
                // SAFETY: tx was only borrowed in the context
                // that was passed into the function and no longer
                // exists.
                unsafe { &mut *tx.get() },
            )
            .await?;

            tracing::info!(
                version,
                name = %mig.name,
                execution_time = %humantime::Duration::from(execution_time),
                "migration reverted"
            );
        }

        tracing::info!("committing changes");
        tx.into_inner().commit().await?;

        Ok(MigrationSummary {
            old_version: if db_migrations.is_empty() {
                None
            } else {
                Some(db_migrations.len() as _)
            },
            new_version: if target_version == 1 {
                None
            } else {
                Some(target_version - 1)
            },
        })
    }

    /// Revert all applied migrations, if any.
    ///
    /// # Errors
    ///
    /// Uses [`Migrator::revert`], any errors will be propagated.
    pub async fn revert_all(&mut self) -> Result<MigrationSummary, Error> {
        self.revert(1).await
    }

    /// Forcibly set a given migration version in the database.
    /// No migrations will be applied or reverted.
    ///
    /// This function should be considered (almost) idempotent, and repeatedly calling it
    /// should result in the same state. Some database-specific values can change, such as timestamps.
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
    pub async fn force_version(&mut self, version: u64) -> Result<MigrationSummary, Error> {
        self.conn.ensure_migrations_table(&self.table).await?;

        let db_migrations = self.conn.list_migrations(&self.table).await?;

        if version == 0 {
            self.conn.clear_migrations(&self.table).await?;
            return Ok(MigrationSummary {
                old_version: if db_migrations.is_empty() {
                    None
                } else {
                    Some(db_migrations.len() as _)
                },
                new_version: None,
            });
        }

        self.local_migration(version)?;

        let migrations = self
            .migrations
            .iter()
            .enumerate()
            .take_while(|(idx, _)| *idx < version as usize);

        self.conn.clear_migrations(&self.table).await?;

        let tx = UnsafeCell::new(self.conn.begin().await?);

        for (idx, mig) in migrations {
            let mig_version = idx as u64 + 1;

            let hasher = UnsafeCell::new(Sha256::new());

            let ctx = MigrationContext {
                hash_only: true,
                ext: self.extensions.get(),
                hasher: hasher.get(),
                tx: tx.get(),
            };

            (*mig.up)(ctx).await.map_err(|error| Error::Migration {
                name: mig.name.clone(),
                version: mig_version,
                error,
            })?;

            let checksum = hasher.into_inner().finalize().to_vec();

            DB::Connection::add_migration(
                &self.table,
                AppliedMigration {
                    version: mig_version,
                    name: mig.name.clone(),
                    checksum: checksum.into(),
                    execution_time: Duration::default(),
                },
                // SAFETY: tx was only borrowed in the context
                // that was passed into the function and no longer
                // exists.
                unsafe { &mut *tx.get() },
            )
            .await?;

            tracing::info!(
                version = idx + 1,
                name = %mig.name,
                "migration forcibly set as applied"
            );
        }

        tracing::info!("committing changes");
        tx.into_inner().commit().await?;

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
        self.conn.ensure_migrations_table(&self.table).await?;
        let migrations = self.conn.list_migrations(&self.table).await?;
        self.check_migrations(&migrations)?;

        if self.options.verify_checksums {
            for res in self.verify_checksums(&migrations).await? {
                res?;
            }
        }

        Ok(())
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

        let checksums = self.verify_checksums(&migrations).await?;

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
                    applied: Some(db),
                    missing_local: false,
                    checksum_ok: checksums.get(idx).map_or(true, Result::is_ok),
                }),
                EitherOrBoth::Left(local) => status.push(MigrationStatus {
                    version,
                    name: local.name.clone().into_owned(),
                    reversible: local.is_reversible(),
                    applied: None,
                    missing_local: false,
                    checksum_ok: checksums.get(idx).map_or(true, Result::is_ok),
                }),
                EitherOrBoth::Right(r) => status.push(MigrationStatus {
                    version: r.version,
                    name: r.name.clone().into_owned(),
                    reversible: false,
                    applied: Some(r),
                    missing_local: true,
                    checksum_ok: checksums.get(idx).map_or(true, Result::is_ok),
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

    fn check_migrations(&mut self, migrations: &[AppliedMigration<'_>]) -> Result<(), Error> {
        if self.migrations.len() < migrations.len() {
            return Err(Error::MissingMigrations {
                local_count: self.migrations.len(),
                db_count: migrations.len(),
            });
        }

        for (idx, (db_migration, local_migration)) in
            migrations.iter().zip(self.migrations.iter()).enumerate()
        {
            let version = idx as u64 + 1;

            if self.options.verify_names && db_migration.name != local_migration.name {
                return Err(Error::NameMismatch {
                    version,
                    local_name: local_migration.name.clone(),
                    db_name: db_migration.name.to_string().into(),
                });
            }
        }

        Ok(())
    }

    async fn verify_checksums(
        &mut self,
        migrations: &[AppliedMigration<'_>],
    ) -> Result<Vec<Result<(), Error>>, Error> {
        let mut results = Vec::with_capacity(self.migrations.len());

        let local_migrations = self.migrations.iter();

        let tx = UnsafeCell::new(self.conn.begin().await?);

        for (idx, mig) in local_migrations.enumerate() {
            let mig_version = idx as u64 + 1;

            let hasher = UnsafeCell::new(Sha256::new());

            let ctx = MigrationContext {
                hash_only: true,
                ext: self.extensions.get(),
                hasher: hasher.get(),
                tx: tx.get(),
            };

            (*mig.up)(ctx).await.map_err(|error| Error::Migration {
                name: mig.name.clone(),
                version: mig_version,
                error,
            })?;

            let checksum = hasher.into_inner().finalize().to_vec();

            if let Some(db_mig) = migrations.get(idx) {
                if db_mig.checksum == checksum {
                    results.push(Ok(()));
                } else {
                    results.push(Err(Error::ChecksumMismatch {
                        version: mig_version,
                        local_checksum: checksum.clone().into(),
                        db_checksum: db_mig.checksum.clone().into_owned().into(),
                    }));
                }
            }
        }

        tx.into_inner().rollback().await?;

        Ok(results)
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
    /// Information about the migration in the database.
    pub applied: Option<db::AppliedMigration<'static>>,
    /// Whether the migration is found in the database,
    /// but missing locally.
    pub missing_local: bool,
    /// Whether the checksum matches the database checksum.
    pub checksum_ok: bool,
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
    Sqlite,
    Any,
}

impl DatabaseType {
    fn sqlx_type(self) -> &'static str {
        match self {
            DatabaseType::Postgres => "Postgres",
            DatabaseType::Sqlite => "Sqlite",
            DatabaseType::Any => "Any",
        }
    }
}

impl FromStr for DatabaseType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "postgres" => Ok(Self::Postgres),
            "sqlite" => Ok(Self::Sqlite),
            "any" => Ok(Self::Any),
            db => Err(anyhow::anyhow!("invalid database type `{}`", db)),
        }
    }
}
