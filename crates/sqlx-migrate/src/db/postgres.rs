use std::{borrow::Cow, time::Duration};

use async_trait::async_trait;
use sqlx::{query, query_as, query_scalar, PgConnection};

use super::AppliedMigration;

#[async_trait(?Send)]
impl super::Migrations for sqlx::PgConnection {
    async fn ensure_migrations_table(&mut self, table_name: &str) -> Result<(), sqlx::Error> {
        query(&format!(
            r#"
                CREATE TABLE IF NOT EXISTS {} (
                    version BIGINT PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_on TIMESTAMPTZ NOT NULL DEFAULT now(),
                    checksum BYTEA NOT NULL,
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
        let database_name = current_database(self).await?;
        let lock_id = generate_lock_id(&database_name);

        // create an application lock over the database
        // this function will not return until the lock is acquired

        // https://www.postgresql.org/docs/current/explicit-locking.html#ADVISORY-LOCKS
        // https://www.postgresql.org/docs/current/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS-TABLE

        // language=SQL
        let _ = query("SELECT pg_advisory_lock($1)")
            .bind(lock_id)
            .execute(self)
            .await?;

        Ok(())
    }

    async fn unlock(&mut self) -> Result<(), sqlx::Error> {
        let database_name = current_database(self).await?;
        let lock_id = generate_lock_id(&database_name);

        // language=SQL
        let _ = query("SELECT pg_advisory_unlock($1)")
            .bind(lock_id)
            .execute(self)
            .await?;

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
                INSERT INTO {} ( version, name, checksum, execution_time )
                VALUES ( $1, $2, $3, $4 )
            "#,
            table_name
        ))
        .bind(migration.version as i64)
        .bind(&*migration.name.clone())
        .bind(&*migration.checksum.clone())
        .bind(migration.execution_time.as_nanos() as i64)
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

async fn current_database(conn: &mut PgConnection) -> Result<String, sqlx::Error> {
    query_scalar("SELECT current_database()")
        .fetch_one(conn)
        .await
}

// inspired from rails: https://github.com/rails/rails/blob/6e49cc77ab3d16c06e12f93158eaf3e507d4120e/activerecord/lib/active_record/migration.rb#L1308
fn generate_lock_id(database_name: &str) -> i64 {
    const CRC_IEEE: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    // 0x20871d5f chosen by fair dice roll
    0x20871d5f * (CRC_IEEE.checksum(database_name.as_bytes()) as i64)
}
