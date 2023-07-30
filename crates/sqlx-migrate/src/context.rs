use sha2::{Digest, Sha256};
use state::TypeMap;
use std::{any::Any, borrow::BorrowMut, sync::Arc};

use sqlx::{Database, Executor};

pub struct MigrationContext<Db>
where
    Db: Database,
{
    pub(crate) hash_only: bool,
    pub(crate) hasher: Sha256,
    pub(crate) conn: Db::Connection,
    pub(crate) ext: Arc<TypeMap![Send + Sync]>,
}

impl<Db: std::fmt::Debug> std::fmt::Debug for MigrationContext<Db>
where
    Db: Database,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationContext")
            .field("hash_only", &self.hash_only)
            .field("hasher", &self.hasher)
            .field("ext", &self.ext)
            .finish_non_exhaustive()
    }
}

impl<Db> MigrationContext<Db>
where
    Db: Database,
{
    /// Return an executor that can execute queries.
    ///
    /// Currently this just re-borrows self.
    pub fn tx(&mut self) -> &mut Self {
        self
    }

    /// Get an extension.
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.ext.try_get()
    }
}

// Implementing this in a generic way confuses the hell out of rustc,
// so instead this is copy/pasted for all supported backends.
#[cfg(feature = "postgres")]
impl<'c> Executor<'c> for &'c mut MigrationContext<sqlx::Postgres> {
    type Database = sqlx::Postgres;

    fn fetch_many<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<
        'e,
        Result<
            itertools::Either<
                <Self::Database as Database>::QueryResult,
                <Self::Database as Database>::Row,
            >,
            sqlx::Error,
        >,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_many("");
        }

        self.conn.borrow_mut().fetch_many(query)
    }

    fn fetch_optional<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<Option<<Self::Database as Database>::Row>, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return Box::pin(async move { Ok(None) });
        }

        self.conn.borrow_mut().fetch_optional(query)
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        parameters: &'e [<Self::Database as Database>::TypeInfo],
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as sqlx::database::HasStatement<'q>>::Statement, sqlx::Error>,
    >
    where
        'c: 'e,
    {
        self.hasher.update(sql);
        self.conn.borrow_mut().prepare_with(sql, parameters)
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> futures_core::future::BoxFuture<'e, Result<sqlx::Describe<Self::Database>, sqlx::Error>>
    where
        'c: 'e,
    {
        self.hasher.update(sql);
        self.conn.borrow_mut().describe(sql)
    }

    fn execute<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as Database>::QueryResult, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().execute("");
        }

        self.conn.borrow_mut().execute(query)
    }

    fn execute_many<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<
        'e,
        Result<<Self::Database as Database>::QueryResult, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().execute_many("");
        }

        self.conn.borrow_mut().execute_many(query)
    }

    fn fetch<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<'e, Result<<Self::Database as Database>::Row, sqlx::Error>>
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch("");
        }

        self.conn.borrow_mut().fetch(query)
    }

    fn fetch_all<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<Vec<<Self::Database as Database>::Row>, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_all("");
        }

        self.conn.borrow_mut().fetch_all(query)
    }

    fn fetch_one<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<'e, Result<<Self::Database as Database>::Row, sqlx::Error>>
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_one("");
        }

        self.conn.borrow_mut().fetch_one(query)
    }

    fn prepare<'e, 'q: 'e>(
        self,
        query: &'q str,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as sqlx::database::HasStatement<'q>>::Statement, sqlx::Error>,
    >
    where
        'c: 'e,
    {
        self.hasher.update(query);
        self.conn.borrow_mut().prepare(query)
    }
}

// Implementing this in a generic way confuses the hell out of rustc,
// so instead this is copy/pasted for all supported backends.
#[cfg(feature = "sqlite")]
impl<'c> Executor<'c> for &'c mut MigrationContext<sqlx::Sqlite> {
    type Database = sqlx::Sqlite;

    fn fetch_many<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<
        'e,
        Result<
            itertools::Either<
                <Self::Database as Database>::QueryResult,
                <Self::Database as Database>::Row,
            >,
            sqlx::Error,
        >,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_many("");
        }

        self.conn.borrow_mut().fetch_many(query)
    }

    fn fetch_optional<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<Option<<Self::Database as Database>::Row>, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return Box::pin(async move { Ok(None) });
        }

        self.conn.borrow_mut().fetch_optional(query)
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        parameters: &'e [<Self::Database as Database>::TypeInfo],
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as sqlx::database::HasStatement<'q>>::Statement, sqlx::Error>,
    >
    where
        'c: 'e,
    {
        self.hasher.update(sql);
        self.conn.borrow_mut().prepare_with(sql, parameters)
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> futures_core::future::BoxFuture<'e, Result<sqlx::Describe<Self::Database>, sqlx::Error>>
    where
        'c: 'e,
    {
        self.hasher.update(sql);
        self.conn.borrow_mut().describe(sql)
    }

    fn execute<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as Database>::QueryResult, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().execute("");
        }

        self.conn.borrow_mut().execute(query)
    }

    fn execute_many<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<
        'e,
        Result<<Self::Database as Database>::QueryResult, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().execute_many("");
        }

        self.conn.borrow_mut().execute_many(query)
    }

    fn fetch<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::stream::BoxStream<'e, Result<<Self::Database as Database>::Row, sqlx::Error>>
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch("");
        }

        self.conn.borrow_mut().fetch(query)
    }

    fn fetch_all<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<Vec<<Self::Database as Database>::Row>, sqlx::Error>,
    >
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_all("");
        }

        self.conn.borrow_mut().fetch_all(query)
    }

    fn fetch_one<'e, 'q: 'e, E: 'q>(
        self,
        query: E,
    ) -> futures_core::future::BoxFuture<'e, Result<<Self::Database as Database>::Row, sqlx::Error>>
    where
        'c: 'e,
        E: sqlx::Execute<'q, Self::Database>,
    {
        self.hasher.update(query.sql());

        if self.hash_only {
            return self.conn.borrow_mut().fetch_one("");
        }

        self.conn.borrow_mut().fetch_one(query)
    }

    fn prepare<'e, 'q: 'e>(
        self,
        query: &'q str,
    ) -> futures_core::future::BoxFuture<
        'e,
        Result<<Self::Database as sqlx::database::HasStatement<'q>>::Statement, sqlx::Error>,
    >
    where
        'c: 'e,
    {
        self.hasher.update(query);
        self.conn.borrow_mut().prepare(query)
    }
}
