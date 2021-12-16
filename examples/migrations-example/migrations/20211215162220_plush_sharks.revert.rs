use barrel::{backend::Pg, types};
use sqlx::{query, query_as, Executor, Postgres, Transaction};
use sqlx_migrate::prelude::*;
use std::borrow::BorrowMut;

/// Reverts migration `plush_sharks` in the given transaction.
//
// Do not modify the function name.
// Do not modify the signature with the exception of the SQLx database type.
pub async fn revert_plush_sharks(tx: &mut Transaction<'_, Postgres>) -> Result<(), MigrationError> {
    let mut m = barrel::Migration::new();
    m.change_table("users", |t| {
        t.add_column("owns_plush_sharks", types::boolean().default(false));
    });

    tx.execute(m.make::<Pg>().as_ref()).await?;

    let mut users_with_sharks: Vec<i32> = query_as::<_, (i32,)>(
        r#"
        SELECT
            owner
        FROM
            plush_sharks
        "#,
    )
    .fetch_all(tx.borrow_mut())
    .await?
    .into_iter()
    .map(|v| v.0)
    .collect();

    tx.execute("DROP TABLE plush_sharks").await?;

    users_with_sharks.sort_unstable();
    users_with_sharks.dedup();

    for user_id in users_with_sharks {
        query(
            r#"
            UPDATE
                users
            SET
                owns_plush_sharks = TRUE
            WHERE
                user_id = $1
            "#,
        )
        .bind(user_id)
        .execute(tx.borrow_mut())
        .await?;
    }

    Ok(())
}
