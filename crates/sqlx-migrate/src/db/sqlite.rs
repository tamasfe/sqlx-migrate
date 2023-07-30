use async_trait::async_trait;
use sqlx::{query, query_as};
use std::{borrow::Cow, time::Duration};
use time::OffsetDateTime;

use super::AppliedMigration;

#[async_trait(?Send)]
impl super::Migrations for sqlx::SqliteConnection {
    async fn ensure_migrations_table(&mut self, table_name: &str) -> Result<(), sqlx::Error> {
        query(&format!(
            r#"
                CREATE TABLE IF NOT EXISTS {} (
                    version BIGINT PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_on INTEGER NOT NULL,
                    checksum BLOB NOT NULL,
                    execution_time BIGINT NOT NULL
                );
                "#,
            table_name
        ))
        .execute(self)
        .await?;

        Ok(())
    }

    async fn lock(&mut self) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn unlock(&mut self) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn list_migrations(
        &mut self,
        table_name: &str,
    ) -> Result<Vec<super::AppliedMigration<'static>>, sqlx::Error> {
        let rows: Vec<(i64, String, Vec<u8>, i64)> = query_as(&format!(
            r#"
            SELECT
                version,
                name,
                checksum,
                execution_time
            FROM
                {}
            ORDER BY version
            "#,
            table_name
        ))
        .fetch_all(self)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| AppliedMigration {
                version: row.0 as u64,
                name: Cow::Owned(row.1),
                checksum: Cow::Owned(row.2),
                execution_time: Duration::from_nanos(row.3 as _),
            })
            .collect())
    }

    async fn add_migration(
        &mut self,
        table_name: &str,
        migration: super::AppliedMigration<'static>,
    ) -> Result<(), sqlx::Error> {
        query(&format!(
            r#"
                INSERT INTO {} ( version, name, checksum, execution_time, applied_on )
                VALUES ( $1, $2, $3, $4, $5 )
            "#,
            table_name
        ))
        .bind(migration.version as i64)
        .bind(&*migration.name.clone())
        .bind(&*migration.checksum.clone())
        .bind(migration.execution_time.as_nanos() as i64)
        .bind(OffsetDateTime::now_utc().unix_timestamp())
        .execute(self)
        .await?;

        Ok(())
    }

    async fn remove_migration(
        &mut self,
        table_name: &str,
        version: u64,
    ) -> Result<(), sqlx::Error> {
        query(&format!(r#"DELETE FROM {} WHERE version = $1"#, table_name))
            .bind(version as i64)
            .execute(self)
            .await?;

        Ok(())
    }

    async fn clear_migrations(&mut self, table_name: &str) -> Result<(), sqlx::Error> {
        query(&format!("TRUNCATE {}", table_name))
            .execute(self)
            .await?;
        Ok(())
    }
}
