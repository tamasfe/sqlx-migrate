# Example Migrations

An example self-contained crate that generates and embeds postgres database migrations and also provides a CLI for managing them.
The idea is that you can build it once, and apply/revert the migrations in any database without any additional required tools.

It contains two migrations inspired by the `barrel` library example.

## Usage

Simply run `cargo run --release --bin migrations-example` to explore the available commands and features.

To apply all migrations:

- create or pick a database, and set the `DATABASE_URL` environment variable (or use the `--database-url` argument).
- run `cargo run --release --bin migrations-example -- migrate` to apply all migrations.
- run `cargo run --release --bin migrations-example -- status` to verify that they are applied.

To revert all migrations:

- run `cargo run --release --bin migrations-example -- revert --do-as-i-say`.

In order to add a new migration run `cargo run --bin migrations-example -- add example`, modifying migrations is only possible in debug builds right now since rebuilds are required anyway, and helps avoiding accidental fs modifications if the migration cli needs to be portable.

## The Structure

To achieve this behaviour in your projects there are a few steps required:

First add `sqlx`, `sqlx-migrate` to your dependencies, for example:

```toml
[dependencies]
sqlx = { version = "0.5.9", features = ["runtime-tokio-rustls", "postgres"] }
sqlx-migrate = { version = "0.1.0", features = ["cli"] }

[build-dependencies]
sqlx-migrate = { version = "0.1.0", features = ["generate"] }
```

Then create a `build.rs` file with the following content:

```rs
use std::path::Path;
use sqlx_migrate::{generate, DatabaseType};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    generate(
        root.join("migrations"),
        root.join("src/generated.rs"),
        DatabaseType::Postgres,
    );
}
```

This will generate `src/generated.rs` based on the given `migrations` directory.
The migrations directory has to be created manually, but all generated source directories will be created.

Make sure that the generated module is included in your module tree, you can also use `lib.rs` if you intend to use the package for generated migrations.

Finally add the `sqlx-migrate` CLI, for example in `bin/migrate.rs` with the following content:

```rs
use std::path::Path;

fn main() {
    sqlx_migrate::cli::run(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
        migrations_example::generated::migrations(),
    );
}
```

And add the entry in `Cargo.toml`:

```toml
[[bin]]
name = "migrations-example"
path = "bin/migrate.rs"
```
