//! Database-specific items.

#[cfg(feature = "postgres")]
mod postgres;

use std::{borrow::Cow, time::Duration};
use async_trait::async_trait;
use sqlx::{Connection, Transaction};

#[derive(Debug, Clone)]
pub struct AppliedMigration<'m> {
    pub version: u64,
    pub name: Cow<'m, str>,
    pub checksum: Cow<'m, [u8]>,
    pub execution_time: Duration,
}

#[async_trait(?Send)]
pub trait Migrations: Connection {
    #[must_use]
    async fn ensure_migrations_table(&mut self, table_name: &str) -> Result<(), sqlx::Error>;

    // Should acquire a database lock so that only one migration process
    // can run at a time. [`Migrate`] will call this function before applying
    // any migrations.
    #[must_use]
    async fn lock(&mut self) -> Result<(), sqlx::Error>;

    // Should release the lock. [`Migrate`] will call this function after all
    // migrations have been run.
    #[must_use]
    async fn unlock(&mut self) -> Result<(), sqlx::Error>;

    // Return the ordered list of applied migrations
    #[must_use]
    async fn list_migrations(
        &mut self,
        table_name: &str,
    ) -> Result<Vec<AppliedMigration<'static>>, sqlx::Error>;

    #[must_use]
    async fn add_migration(
        table_name: &str,
        migration: AppliedMigration<'static>,
        tx: &mut Transaction<'_, Self::Database>,
    ) -> Result<(), sqlx::Error>;

    #[must_use]
    async fn remove_migration(
        table_name: &str,
        version: u64,
        tx: &mut Transaction<'_, Self::Database>,
    ) -> Result<(), sqlx::Error>;

    #[must_use]
    async fn clear_migrations(&mut self, table_name: &str) -> Result<(), sqlx::Error>;
}
