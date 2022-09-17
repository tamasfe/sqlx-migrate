use sha2::{Digest, Sha256};
use std::{
    any::{Any, TypeId},
    borrow::BorrowMut,
    collections::HashMap,
};

use sqlx::{Database, Executor, Transaction};

#[derive(Debug)]
pub struct MigrationContext<'c, Db>
where
    Db: Database,
{
    pub(crate) hash_only: bool,
    pub(crate) hasher: *mut Sha256,
    pub(crate) tx: *mut Transaction<'c, Db>,
    pub(crate) ext: *const Extensions,
}

impl<'c, Db> MigrationContext<'c, Db>
where
    Db: Database,
{
    /// Calling this function will reborrow
    /// the context allowing it to be used multiple times.
    ///
    /// This is no different than [`std::borrow::BorrowMut`],
    /// but without needing to import the trait.
    pub fn tx<'s, 't>(&'t mut self) -> impl Executor<'c, Database = Db> + 's + 't
    where
        's: 't,
        &'s mut Transaction<'c, Db>: Executor<'c, Database = Db> + 's,
    {
        // SAFETY: Self is mutably borrowed.
        MigrationCtxExecutor {
            hash_only: self.hash_only,
            hasher: unsafe { &mut *self.hasher  },
            tx: unsafe { &mut *self.tx  },
        }
    }

    /// Get an extension.
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        // SAFETY: we access the extensions immutably.
        unsafe { (*self.ext).get() }
    }
}

#[derive(Debug)]
struct MigrationCtxExecutor<'c, 't, Db>
where
    Db: Database,
    'c: 't,
    &'t mut Transaction<'c, Db>: Executor<'c, Database = Db>,
{
    pub(crate) hash_only: bool,
    pub(crate) hasher: &'t mut Sha256,
    pub(crate) tx: &'t mut Transaction<'c, Db>,
}

#[allow(clippy::type_repetition_in_bounds)]
impl<'c, 't, Db> Executor<'c> for MigrationCtxExecutor<'c, 't, Db>
where
    Db: Database,
    &'t mut Transaction<'c, Db>: Executor<'c, Database = Db>,
{
    type Database = Db;

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
            return self.tx.borrow_mut().fetch_many("");
        }

        self.tx.borrow_mut().fetch_many(query)
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

        self.tx.borrow_mut().fetch_optional(query)
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
        self.tx.borrow_mut().prepare_with(sql, parameters)
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> futures_core::future::BoxFuture<'e, Result<sqlx::Describe<Self::Database>, sqlx::Error>>
    where
        'c: 'e,
    {
        self.hasher.update(sql);
        self.tx.borrow_mut().describe(sql)
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
            return self.tx.borrow_mut().execute("");
        }

        self.tx.borrow_mut().execute(query)
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
            return self.tx.borrow_mut().execute_many("");
        }

        self.tx.borrow_mut().execute_many(query)
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
            return self.tx.borrow_mut().fetch("");
        }

        self.tx.borrow_mut().fetch(query)
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
            return self.tx.borrow_mut().fetch_all("");
        }

        self.tx.borrow_mut().fetch_all(query)
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
            return self.tx.borrow_mut().fetch_one("");
        }

        self.tx.borrow_mut().fetch_one(query)
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
        self.tx.borrow_mut().prepare(query)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Extensions {
    pub(crate) map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|t| t.downcast_ref::<T>())
    }
}
