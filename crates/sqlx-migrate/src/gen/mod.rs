use crate::DatabaseType;
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, DirEntry},
    path::Path,
};

mod build_rs;

pub use build_rs::generate;

#[must_use]
pub fn migration_modules(migrations_path: &Path) -> TokenStream {
    assert!(
        migrations_path.is_dir(),
        "migrations path must be a directory ({migrations_path:?})",
    );

    let mut modules = quote! {};

    let mut files = fs::read_dir(migrations_path)
        .unwrap()
        .map(Result::unwrap)
        .filter(|file| {
            let file_path = file.path();

            if file_path.is_dir() {
                return false;
            }

            let fname = file.file_name();

            let file_name = fname.to_string_lossy();
            let file_name_lower = file_name.to_ascii_lowercase();

            if !(file_name_lower.ends_with(".migrate.rs")
                || file_name_lower.ends_with(".revert.rs")
                || file_name_lower.ends_with(".migrate.sql")
                || file_name_lower.ends_with(".revert.sql"))
            {
                return false;
            }

            true
        })
        .collect::<Vec<_>>();

    files.sort_by_key(DirEntry::file_name);

    let mut version = 0;

    for file in files {
        let file_path = file.path();

        if file_path.is_dir() {
            continue;
        }

        let fname = file.file_name();

        let file_name = fname.to_string_lossy();
        let file_name_lower = file_name.to_ascii_lowercase();

        if !(file_name_lower.ends_with(".migrate.rs")
            || file_name_lower.ends_with(".revert.rs")
            || file_name_lower.ends_with(".migrate.sql")
            || file_name_lower.ends_with(".revert.sql"))
        {
            continue;
        }

        let split = split_name(&file_name, &file_name_lower);

        let MigrationSplit {
            name,
            kind,
            source,
            date,
        } = split;

        let file_path_str = file_path.to_string_lossy().to_string();

        let docstr = format!(" Created at {date}.");

        if let MigrationKind::Up = kind {
            version += 1;
        }

        let name_ident = Ident::new(
            &format!(
                "_{}_{}_{}",
                version,
                name,
                match kind {
                    MigrationKind::Down => "revert",
                    MigrationKind::Up => "migrate",
                }
            ),
            Span::call_site(),
        );

        match source {
            MigrationSourceKind::Rust => {
                modules.extend(quote! {
                    #[allow(dead_code)]
                    #[allow(clippy::all, clippy::pedantic)]
                    #[path = #file_path_str]
                    #[doc = #docstr]
                    pub mod #name_ident;

                    #[doc(inline)]
                    pub use #name_ident::*;
                });
            }
            MigrationSourceKind::Sql => {
                modules.extend(quote! {
                    #[allow(dead_code)]
                    #[allow(clippy::all, clippy::pedantic)]
                    #[doc = #docstr]
                    pub mod #name_ident {}
                });
            }
        }
    }

    modules
}

// The length of dates before the migration names.
const MIG_DATE_PREFIX_LEN: usize = "20001010235912_".len();

struct Migration {
    date: u64,
    name: String,
    up_fn: Option<TokenStream>,
    down_fn: Option<TokenStream>,
}

#[allow(clippy::too_many_lines)]
#[must_use]
pub fn migrations(db: DatabaseType, migrations_path: &Path) -> TokenStream {
    assert!(
        migrations_path.is_dir(),
        "migrations path must be a directory ({migrations_path:?})",
    );

    // Migrations by their name.
    let mut migrations: HashMap<String, Migration> = HashMap::new();

    let db_ident = format_ident!("{}", db.sqlx_type());

    for file in fs::read_dir(migrations_path).unwrap() {
        let file = file.unwrap();

        let file_path = file.path();

        if file_path.is_dir() {
            continue;
        }

        let fname = file.file_name();

        let file_name = fname.to_string_lossy();
        let file_name_lower = file_name.to_ascii_lowercase();

        if !(file_name_lower.ends_with(".migrate.rs")
            || file_name_lower.ends_with(".revert.rs")
            || file_name_lower.ends_with(".migrate.sql")
            || file_name_lower.ends_with(".revert.sql"))
        {
            continue;
        }

        let split = split_name(&file_name, &file_name_lower);

        let mig = migrations.entry(split.name.clone()).or_insert(Migration {
            date: split.date,
            name: split.name,
            up_fn: None,
            down_fn: None,
        });

        match split.kind {
            MigrationKind::Up => {
                assert!(
                    mig.up_fn.is_none(),
                    "duplicate up migration for {}",
                    &mig.name
                );

                let source_string = fs::read_to_string(&file_path).unwrap();

                let mut hasher = Sha256::new();
                hasher.update(source_string.as_bytes());

                let file_path_str = file_path.to_string_lossy().to_string();

                let mig_ident = Ident::new(&mig.name, Span::call_site());

                match split.source {
                    MigrationSourceKind::Rust => {
                        mig.up_fn = Some(quote! {
                            #[path = #file_path_str]
                            mod #mig_ident;

                            #mig_ident::#mig_ident(ctx).await?;

                            Ok(())
                        });
                    }
                    MigrationSourceKind::Sql => {
                        mig.up_fn = Some(quote! {
                            use sqlx::Executor;
                            let ctx: &mut sqlx_migrate::prelude::MigrationContext<sqlx::#db_ident> = ctx;
                            ctx.tx().execute(include_str!(#file_path_str)).await?;
                            Ok(())
                        });
                    }
                }
            }
            MigrationKind::Down => {
                assert!(
                    mig.down_fn.is_none(),
                    "duplicate down migration for {}",
                    &mig.name
                );

                let file_path_str = file_path.to_string_lossy().to_string();

                let mig_ident = Ident::new(&format!("revert_{}", &mig.name), Span::call_site());

                match split.source {
                    MigrationSourceKind::Rust => {
                        mig.down_fn = Some(quote! {
                            #[path = #file_path_str]
                            mod #mig_ident;

                            #mig_ident::#mig_ident(ctx).await?;

                            Ok(())
                        });
                    }
                    MigrationSourceKind::Sql => {
                        mig.down_fn = Some(quote! {
                            use sqlx::Executor;
                            let ctx: &mut sqlx_migrate::prelude::MigrationContext<sqlx::#db_ident> = ctx;
                            ctx.tx().execute(include_str!(#file_path_str)).await?;
                            Ok(())
                        });
                    }
                }
            }
        }
    }

    let mut migrations = migrations.into_values().collect::<Vec<_>>();

    migrations.sort_by(|a, b| a.date.cmp(&b.date));

    let mut migration_tokens = quote! {};

    for mig in migrations {
        let Migration {
            date: _,
            name,
            up_fn,
            down_fn,
        } = mig;

        assert!(up_fn.is_some(), "missing up migration for {}", &name);

        migration_tokens.extend(quote! {
            sqlx_migrate::Migration::new(
                #name, |ctx| std::boxed::Box::pin(async move {
                    #up_fn
                })
            )
        });

        if let Some(down) = down_fn {
            migration_tokens.extend(quote! {
                .reversible(|ctx| std::boxed::Box::pin(async move {
                    #down
                })
                )
            });
        }

        migration_tokens.extend(quote!(,));
    }

    quote! {[#migration_tokens]}
}

enum MigrationKind {
    Up,
    Down,
}

enum MigrationSourceKind {
    Rust,
    Sql,
}

struct MigrationSplit {
    date: u64,
    name: String,
    kind: MigrationKind,
    source: MigrationSourceKind,
}

// (full_name, date, name, sql)
fn split_name(file_name: &str, file_name_lower: &str) -> MigrationSplit {
    assert!(
        file_name.is_ascii(),
        "file name must be ASCII ({file_name})",
    );

    assert!(
        file_name.len() >= MIG_DATE_PREFIX_LEN,
        "invalid migration file name ({file_name})",
    );

    let date: u64 = file_name[..MIG_DATE_PREFIX_LEN - 1].parse().unwrap();

    let mut split = file_name_lower[MIG_DATE_PREFIX_LEN..].rsplitn(3, '.');

    let source = match split.next().unwrap() {
        "rs" => MigrationSourceKind::Rust,
        "sql" => MigrationSourceKind::Sql,
        _ => unreachable!(),
    };

    let kind = match split.next().unwrap() {
        "migrate" => MigrationKind::Up,
        "revert" => MigrationKind::Down,
        _ => unreachable!(),
    };

    let name = file_name[MIG_DATE_PREFIX_LEN..]
        .rsplitn(3, '.')
        .nth(2)
        .unwrap()
        .to_string();

    MigrationSplit {
        date,
        name,
        kind,
        source,
    }
}
