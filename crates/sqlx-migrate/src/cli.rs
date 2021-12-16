#![allow(
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    unused_imports,
    dead_code,
    unused_variables
)]
use crate::{db, prelude::*, DEFAULT_MIGRATIONS_TABLE};
use clap::StructOpt;
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use filetime::FileTime;
use regex::Regex;
use sqlx::{Database, ConnectOptions};
use std::{fs, io, path::Path, process, str::FromStr, time::Duration};
use time::{format_description, OffsetDateTime};
use tracing_subscriber::{
    fmt::format::FmtSpan, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    EnvFilter,
};

#[derive(Debug, clap::Parser)]
pub struct Migrate {
    /// Disable colors in messages.
    #[clap(long, global(true))]
    no_colors: bool,
    /// Enable the logging of tracing spans.
    #[clap(long, global(true))]
    verbose: bool,
    /// Force the operation, required for some actions.
    #[clap(long = "do-as-i-say", visible_aliases = &["force"], global(true))]
    force: bool,
    /// Skip verifying checksums.
    #[clap(long, alias = "no-verify-checksum", global(true))]
    no_verify_checksums: bool,
    /// Skip verifying names.
    #[clap(long, alias = "no-verify-name", global(true))]
    no_verify_names: bool,
    /// Log all SQL statements.
    #[clap(long, global(true))]
    log_statements: bool,
    /// Database URL, if not given the `DATABASE_URL` environment variable will be used.
    #[clap(long, visible_alias = "db-url", global(true))]
    database_url: Option<String>,
    /// The name of the migrations table.
    #[clap(long, default_value = DEFAULT_MIGRATIONS_TABLE, global(true))]
    migrations_table: String,
    #[clap(subcommand)]
    operation: Operation,
}

#[derive(Debug, clap::Subcommand)]
enum Operation {
    /// Apply all migrations up to and including the given migration.
    ///
    /// If no migration is given, all migrations are applied.
    #[clap(visible_aliases = &["up", "mig"])]
    Migrate {
        /// Apply all migrations up to and including the migration
        /// with the given name.
        #[clap(long, conflicts_with = "version")]
        name: Option<String>,

        /// Apply all migrations up to and including the migration
        /// with the given version.
        #[clap(long, conflicts_with = "name")]
        version: Option<u64>,
    },
    /// Revert the given migration and all subsequent ones.
    ///
    /// If no migration is set, all applied migrations are reverted.
    #[clap(visible_aliases = &["down", "rev"])]
    Revert {
        /// Revert all migrations after and including the migration
        /// with the given name.
        #[clap(long, conflicts_with = "version")]
        name: Option<String>,

        /// Revert all migrations after and including the migration
        /// the given version.
        #[clap(long, conflicts_with = "name")]
        version: Option<u64>,
    },
    /// Forcibly set a given migration.
    ///
    /// This does not apply nor revert any migrations, and
    /// only overrides migration status.
    #[clap(visible_aliases = &["set"])]
    Force {
        /// Forcibly set the migration with the given name.
        #[clap(conflicts_with = "version", required_unless_present("version"))]
        name: Option<String>,
        /// Forcibly set the migration with the given version.
        #[clap(conflicts_with = "name", required_unless_present("name"))]
        version: Option<u64>,
    },
    /// Verify migrations and print errors.
    #[clap(visible_aliases = &["verify", "validate"])]
    Check {},
    /// List all migrations.
    #[clap(visible_aliases = &["list", "ls", "get"])]
    Status {},
    /// Add a new migration.
    ///
    /// The migrations default to Rust files.
    #[cfg(debug_assertions)]
    #[clap(visible_aliases = &["new"])]
    Add {
        /// Use SQL for the migrations.
        #[clap(long)]
        sql: bool,
        /// Create a "revert" or "down" migration.
        #[clap(long, short = 'r', visible_aliases = &["revert", "revertible"])]
        reversible: bool,
        /// The SQLx type of the database in Rust migrations.
        ///
        /// By default, all migrations will be using `Any`.
        #[clap(
            long = "database",
            visible_aliases = &["db"],
            aliases = &["type"],
            possible_values = &[
                "postgres",
                "any"
            ],
            default_value = "any"
        )]
        ty: DatabaseType,
        /// The name of the migration.
        ///
        /// It must be across all migrations.
        name: String,
    },
}

// Due to crate hierarchy issues this type is duplicated in
// `sqlx-migrate-gen`, if you modify this, make sure
// to update it there as well.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
enum DatabaseType {
    Postgres,
    Any,
}

impl DatabaseType {
    fn sqlx_type(self) -> &'static str {
        match self {
            DatabaseType::Postgres => "Postgres",
            DatabaseType::Any => "Any",
        }
    }
}

impl FromStr for DatabaseType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "postgres" => Ok(Self::Postgres),
            "any" => Ok(Self::Any),
            db => Err(anyhow::anyhow!("invalid database type `{}`", db)),
        }
    }
}

/// Run a CLI application that provides operations with the
/// given migrations.
///
/// When compiled with `debug_assertions`, it additionally allows modifying migrations
/// at the given `migrations_path`.
///
/// Although not required, `migrations` are expected to be originated from `migrations_path`.
///
/// # Panics
///
/// This functon assumes that it has control over the entire application.
///
/// It will happily alter global state (tracing), panic, or terminate the process.
pub fn run<DB>(
    migrations_path: impl AsRef<Path>,
    migrations: impl IntoIterator<Item = Migration<DB>>,
) where
    DB: Database,
    DB::Connection: db::Migrations,
{
    let migrate = Migrate::parse();
    setup_logging(&migrate);

    let migrations = migrations.into_iter().collect::<Vec<_>>();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(execute(migrate, migrations_path.as_ref(), migrations));
}

async fn execute<DB>(migrate: Migrate, migrations_path: &Path, migrations: Vec<Migration<DB>>)
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    let migrator = setup_migrator(&migrate, migrations).await;

    match &migrate.operation {
        Operation::Migrate { name, version } => {
            do_migrate(&migrate, migrator, name.as_deref(), *version).await;
        }
        Operation::Revert { name, version } => {
            revert(&migrate, migrator, name.as_deref(), *version).await;
        }
        Operation::Force { name, version } => {
            force(&migrate, migrator, name.as_deref(), *version).await;
        }
        Operation::Check {} => check(&migrate, migrator).await,
        Operation::Status {} => {
            log_status(&migrate, migrator).await;
        }
        #[cfg(debug_assertions)]
        Operation::Add {
            sql,
            reversible,
            name,
            ty,
        } => add(&migrate, migrations_path, *sql, *reversible, name, *ty),
    }
}

async fn check<DB>(
    _migrate: &Migrate,
    mut migrator: Migrator<DB>,
) where
    DB: Database,
    DB::Connection: db::Migrations,
{
    match migrator.check_migrations().await {
        Ok(_) => {
            tracing::info!("No issues found");
        },
        Err(err) => {
            tracing::error!(error = %err, "error verifying migrations");
            process::exit(1);
        },
    }
}

#[cfg(debug_assertions)]
fn add(
    _migrate: &Migrate,
    migrations_path: &Path,
    sql: bool,
    reversible: bool,
    name: &str,
    ty: DatabaseType,
) {
    let now = OffsetDateTime::now_utc();

    let now_formatted = now
        .format(&format_description::parse("[year][month][day][hour][minute][second]").unwrap())
        .unwrap();

    if !migrations_path.is_dir() {
        tracing::error!("migrations path must be a directory");
        process::exit(1);
    }

    let re = Regex::new("[A-Za-z_][A-Za-z_0-9]*").unwrap();

    if !re.is_match(name) {
        tracing::error!(name, "invalid migration name");
        process::exit(1);
    }

    if sql {
        let up_filename = format!("{}_{}.migrate.sql", &now_formatted, name);

        if let Err(error) = fs::write(
            migrations_path.join(&up_filename),
            &format!(
                r#"-- Migration SQL for {}
"#,
                name
            ),
        ) {
            tracing::error!(error = %error, path = ?migrations_path.join(&up_filename), "failed to write file");
            process::exit(1);
        }

        if reversible {
            let down_filename = format!("{}_{}.revert.sql", &now_formatted, name);
            if let Err(error) = fs::write(
                migrations_path.join(&down_filename),
                &format!(
                    r#"-- Revert SQL for {}
"#,
                    name
                ),
            ) {
                tracing::error!(error = %error, path = ?migrations_path.join(&down_filename), "failed to write file");
                process::exit(1);
            }
        }

        tracing::info!(name, "added migration");
    } else {
        let up_filename = format!("{}_{}.migrate.rs", &now_formatted, name);

        let sqlx_type = ty.sqlx_type();

        if let Err(error) = fs::write(
            migrations_path.join(&up_filename),
            &format!(
                r#"use sqlx::{{{ty}, Transaction}};
use sqlx_migrate::prelude::*;

/// Executes migration `{name}` in the given transaction.
//
// Do not modify the function name.
// Do not modify the signature with the exception of the SQLx database type.
pub async fn {name}(tx: &mut Transaction<'_, {ty}>) -> Result<(), MigrationError> {{
    // write your migration operations here
    todo!()
}}
"#,
                name = name,
                ty = sqlx_type
            ),
        ) {
            tracing::error!(error = %error, path = ?migrations_path.join(&up_filename), "failed to write file");
            process::exit(1);
        }

        if reversible {
            let down_filename = format!("{}_{}.revert.rs", &now_formatted, name);

            if let Err(error) = fs::write(
                migrations_path.join(&down_filename),
                &format!(
                    r#"use sqlx::{{{ty}, Transaction}};
use sqlx_migrate::prelude::*;

/// Reverts migration `{name}` in the given transaction.
//
// Do not modify the function name.
// Do not modify the signature with the exception of the SQLx database type.
pub async fn revert_{name}(tx: &mut Transaction<'_, {ty}>) -> Result<(), MigrationError> {{
    // write your revert operations here
    todo!()
}}
"#,
                    name = name,
                    ty = sqlx_type
                ),
            ) {
                tracing::error!(error = %error, path = ?migrations_path.join(&down_filename), "failed to write file");
                process::exit(1);
            }
        }
    }

    if let Err(err) = filetime::set_file_mtime(migrations_path, FileTime::now()) {
        tracing::debug!(error = %err, "error updating the migrations directory");
    }
}

async fn do_migrate<DB>(
    _migrate: &Migrate,
    mut migrator: Migrator<DB>,
    name: Option<&str>,
    version: Option<u64>,
) where
    DB: Database,
    DB::Connection: db::Migrations,
{
    let version = match version {
        Some(v) => Some(v),
        None => match name {
            Some(name) => {
                if let Some((idx, _)) = migrator
                    .local_migrations()
                    .iter()
                    .enumerate()
                    .find(|mig| mig.1.name() == name)
                {
                    Some(idx as u64 + 1)
                } else {
                    tracing::error!(name = name, "migration not found");
                    process::exit(1);
                }
            }
            None => None,
        },
    };

    match version {
        Some(version) => match migrator.migrate(version).await {
            Ok(s) => print_summary(&s),
            Err(error) => {
                tracing::error!(error = %error, "error applying migrations");
                process::exit(1);
            }
        },
        None => match migrator.migrate_all().await {
            Ok(s) => print_summary(&s),
            Err(error) => {
                tracing::error!(error = %error, "error applying migrations");
                process::exit(1);
            }
        },
    }
}

async fn revert<DB>(
    migrate: &Migrate,
    mut migrator: Migrator<DB>,
    name: Option<&str>,
    version: Option<u64>,
) where
    DB: Database,
    DB::Connection: db::Migrations,
{
    if !migrate.force {
        tracing::error!("the `--do-as-i-say` or `--force` flag is required for this operation");
        process::exit(1);
    }

    let version = match version {
        Some(v) => Some(v),
        None => match name {
            Some(name) => {
                if let Some((idx, _)) = migrator
                    .local_migrations()
                    .iter()
                    .enumerate()
                    .find(|mig| mig.1.name() == name)
                {
                    Some(idx as u64 + 1)
                } else {
                    tracing::error!(name = name, "migration not found");
                    process::exit(1);
                }
            }
            None => None,
        },
    };

    match version {
        Some(version) => match migrator.revert(version).await {
            Ok(s) => print_summary(&s),
            Err(error) => {
                tracing::error!(error = %error, "error reverting migrations");
                process::exit(1);
            }
        },
        None => match migrator.revert_all().await {
            Ok(s) => print_summary(&s),
            Err(error) => {
                tracing::error!(error = %error, "error reverting migrations");
                process::exit(1);
            }
        },
    }
}

async fn force<DB>(
    migrate: &Migrate,
    mut migrator: Migrator<DB>,
    name: Option<&str>,
    version: Option<u64>,
) where
    DB: Database,
    DB::Connection: db::Migrations,
{
    if !migrate.force {
        tracing::error!("the `--do-as-i-say` or `--force` flag is required for this operation");
        process::exit(1);
    }

    let version = match version {
        Some(v) => v,
        None => {
            if let Some((idx, _)) = migrator
                .local_migrations()
                .iter()
                .enumerate()
                .find(|mig| mig.1.name() == name.unwrap())
            {
                idx as u64 + 1
            } else {
                tracing::error!(name = name.unwrap(), "migration not found");
                process::exit(1);
            }
        }
    };

    match migrator.force_version(version).await {
        Ok(s) => print_summary(&s),
        Err(error) => {
            tracing::error!(error = %error, "error updating migrations");
            process::exit(1);
        }
    }
}

async fn log_status<DB>(_migrate: &Migrate, mut migrator: Migrator<DB>)
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    fn mig_ok(status: &MigrationStatus) -> bool {
        if status.missing_local {
            return false;
        }

        match &status.applied {
            Some(applied) => {
                status.checksum == *applied.checksum
                    && status.name == applied.name
                    && status.version == applied.version
            }
            None => true,
        }
    }

    let status = match migrator.status().await {
        Ok(s) => s,
        Err(error) => {
            tracing::error!(error = %error, "error retrieving migration status");
            process::exit(1);
        }
    };

    let all_valid = status.iter().all(mig_ok);

    let mut table = Table::new();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(Vec::from([
            Cell::new("Version").set_alignment(CellAlignment::Center),
            Cell::new("Name").set_alignment(CellAlignment::Center),
            Cell::new("Applied").set_alignment(CellAlignment::Center),
            Cell::new("Valid").set_alignment(CellAlignment::Center),
            Cell::new("Revertible").set_alignment(CellAlignment::Center),
        ]));

    for mig in status {
        let ok = mig_ok(&mig);

        table.add_row(Vec::from([
            Cell::new(mig.version.to_string().as_str()).set_alignment(CellAlignment::Center),
            Cell::new(&mig.name).set_alignment(CellAlignment::Center),
            Cell::new(if mig.applied.is_some() { "x" } else { "" })
                .set_alignment(CellAlignment::Center),
            Cell::new(if ok { "x" } else { "INVALID" }).set_alignment(CellAlignment::Center),
            Cell::new(if mig.reversible { "x" } else { "" }).set_alignment(CellAlignment::Center),
        ]));
    }

    println!("{}", table);

    if !all_valid {
        process::exit(1);
    }
}

fn print_summary(summary: &MigrationSummary) {
    let mut table = Table::new();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(Vec::from([
            Cell::new("Old Version").set_alignment(CellAlignment::Center),
            Cell::new("New Version").set_alignment(CellAlignment::Center),
            Cell::new("Applied Migrations").set_alignment(CellAlignment::Center),
            Cell::new("Reverted Migrations").set_alignment(CellAlignment::Center),
        ]));

    let mut s = Vec::<Cell>::new();

    s.push(match summary.old_version {
        Some(v) => Cell::new(v.to_string()).set_alignment(CellAlignment::Center),
        None => "".into(),
    });

    s.push(match summary.new_version {
        Some(v) => Cell::new(v.to_string()).set_alignment(CellAlignment::Center),
        None => "".into(),
    });

    s.push(match (summary.old_version, summary.new_version) {
        (Some(old), Some(new)) => {
            if new >= old {
                Cell::new((new - old).to_string()).set_alignment(CellAlignment::Center)
            } else {
                "".into()
            }
        }
        (None, Some(new)) => Cell::new(new.to_string()).set_alignment(CellAlignment::Center),
        (_, None) => "".into(),
    });

    s.push(match (summary.old_version, summary.new_version) {
        (Some(old), Some(new)) => {
            if new <= old {
                Cell::new((old - new).to_string()).set_alignment(CellAlignment::Center)
            } else {
                "".into()
            }
        }
        (Some(old), None) => Cell::new(old.to_string()).set_alignment(CellAlignment::Center),
        (None, _) => "".into(),
    });

    table.add_row(s);

    eprintln!("{}", table);
}

async fn setup_migrator<DB>(migrate: &Migrate, migrations: Vec<Migration<DB>>) -> Migrator<DB>
where
    DB: Database,
    DB::Connection: db::Migrations,
{
    let db_url = match &migrate.database_url {
        Some(s) => s.clone(),
        None => {
            if let Ok(url) = std::env::var("DATABASE_URL") {
                url
            } else {
                tracing::error!("`DATABASE_URL` environment variable or `--database-url` argument is required");
                process::exit(1);
            }
        }
    };

    let mut options =
        match db_url.parse::<<<DB as Database>::Connection as sqlx::Connection>::Options>() {
            Ok(opts) => opts,
            Err(err) => {
                tracing::error!(error = %err, "invalid database URL");
                process::exit(1);
            }
        };
    
    if migrate.log_statements {
        options.log_statements("INFO".parse().unwrap());
        options.log_slow_statements("WARN".parse().unwrap(), Duration::from_secs(1));
    } else {
        options.disable_statement_logging();
    }

    match Migrator::connect_with(&options).await {
        Ok(mut mig) => {
            mig.set_options(MigratorOptions {
                verify_checksums: !migrate.no_verify_checksums,
                verify_names: !migrate.no_verify_names,
            });

            if !migrate.migrations_table.is_empty() {
                mig.set_migrations_table(&migrate.migrations_table);
            }

            mig.add_migrations(migrations);

            mig
        }
        Err(err) => {
            tracing::error!(error = %err, "failed to create database connection");
            process::exit(1);
        }
    }
}

fn setup_logging(migrate: &Migrate) {
    let format = tracing_subscriber::fmt::format().with_ansi(colors(migrate));

    let verbose = migrate.verbose;

    let span_events = if verbose {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::CLOSE
    };

    let registry = tracing_subscriber::registry();

    let env_filter = match EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(_) => EnvFilter::default().add_directive(tracing::Level::INFO.into()),
    };

    if verbose {
        registry
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(io::stderr)
                    .with_span_events(span_events)
                    .event_format(format.pretty()),
            )
            .init();
    } else {
        registry
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(io::stderr)
                    .with_span_events(span_events)
                    .event_format(format),
            )
            .init();
    }
}

fn colors(matches: &Migrate) -> bool {
    if matches.no_colors {
        return false;
    }

    atty::is(atty::Stream::Stdout)
}
