pub use sqlx_migrate::prelude::*;
#[allow(dead_code)]
#[allow(clippy::all, clippy::pedantic)]
/// Created at 20211215161742.
pub mod _1_initial_migration_migrate {}
#[allow(dead_code)]
#[allow(clippy::all, clippy::pedantic)]
/// Created at 20211215161742.
pub mod _1_initial_migration_revert {}
#[allow(dead_code)]
#[allow(clippy::all, clippy::pedantic)]
#[path = "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215162220_plush_sharks.migrate.rs"]
/// Created at 20211215162220.
pub mod _2_plush_sharks_migrate;
#[doc(inline)]
pub use _2_plush_sharks_migrate::*;
#[allow(dead_code)]
#[allow(clippy::all, clippy::pedantic)]
#[path = "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215162220_plush_sharks.revert.rs"]
/// Created at 20211215162220.
pub mod _2_plush_sharks_revert;
#[doc(inline)]
pub use _2_plush_sharks_revert::*;
/// All the migrations.
pub fn migrations() -> impl IntoIterator<Item = Migration<sqlx::Postgres>> {
    [
        sqlx_migrate::Migration::new(
                "initial_migration",
                |ctx| std::boxed::Box::pin(async move {
                    use sqlx::Executor;
                    let ctx: &mut sqlx_migrate::prelude::MigrationContext<
                        sqlx::Postgres,
                    > = ctx;
                    ctx.tx()
                        .execute(
                            include_str!(
                                "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215161742_initial_migration.migrate.sql"
                            ),
                        )
                        .await?;
                    Ok(())
                }),
            )
            .reversible(|ctx| std::boxed::Box::pin(async move {
                use sqlx::Executor;
                let ctx: &mut sqlx_migrate::prelude::MigrationContext<sqlx::Postgres> = ctx;
                ctx.tx()
                    .execute(
                        include_str!(
                            "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215161742_initial_migration.revert.sql"
                        ),
                    )
                    .await?;
                Ok(())
            })),
        sqlx_migrate::Migration::new(
                "plush_sharks",
                |ctx| std::boxed::Box::pin(async move {
                    #[path = "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215162220_plush_sharks.migrate.rs"]
                    mod plush_sharks;
                    plush_sharks::plush_sharks(ctx).await?;
                    Ok(())
                }),
            )
            .reversible(|ctx| std::boxed::Box::pin(async move {
                #[path = "/home/tamasfe/work/opensauce/sqlx-migrate/examples/migrations-example/migrations/20211215162220_plush_sharks.revert.rs"]
                mod revert_plush_sharks;
                revert_plush_sharks::revert_plush_sharks(ctx).await?;
                Ok(())
            })),
    ]
}
