use barrel::{
    backend::Pg,
    types::{self, ReferentialAction},
};
use sqlx::{query, query_as, Executor, Postgres};
use sqlx_migrate::prelude::*;

/// Executes migration `plush_sharks` in the given migration context.
///
/// It uses `barrel` for generating the table schema.
//
// Do not modify the function name.
// Do not modify the signature with the exception of the SQLx database type.
pub async fn plush_sharks(mut ctx: MigrationContext<'_, Postgres>) -> Result<(), MigrationError> {
    let users_with_sharks: Vec<(i32,)> = query_as(
        r#"
        SELECT
            user_id
        FROM
            users u
        WHERE
            u.owns_plush_sharks = TRUE
        "#,
    )
    .fetch_all(ctx.tx())
    .await?;

    let mut m = barrel::Migration::new();
    m.create_table("plush_sharks", |t| {
        t.add_column(
            "owner",
            types::foreign(
                "users",
                "user_id",
                ReferentialAction::NoAction,
                ReferentialAction::NoAction,
            ),
        );
        t.add_column("name", types::varchar(255));
        t.add_column("color", types::text());
    });

    m.change_table("users", |t| {
        t.drop_column("owns_plush_sharks");
    });

    ctx.tx().execute(m.make::<Pg>().as_str()).await?;

    for (user_id,) in users_with_sharks {
        // Every user gets a very own plush shark.
        query("INSERT INTO plush_sharks (owner, name, color) VALUES ($1, 'shark', 'blue')")
            .bind(user_id)
            .execute(ctx.tx())
            .await?;
    }

    Ok(())
}
