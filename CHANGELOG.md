# Changelog

## 0.7.0

### Features

- Added `prettyplease` for formatting generated code.

### Other

- Updated dependencies, notably sqlx `0.7` and clap `4` for the CLI.
- Reworked some of the traits and internals due to the changes in sqlx `0.7.0`.
- Reworked API so that migrations receive a `&mut MigrationContext` without additonal lifetimes. This required that we manage our own transactions instead of using sqlx's own transaction type.
- Added the `state` crate for context "extension" management instead of custom type-map implementation.
- Removed all `unsafe` code as it is not needed anymore.

## 0.6.0

### Features

- Instead of `Transaction`, a `MigrationContext` is passed to the migration functions. It is now possible to provide contextual information to migrations, allowing for customizing migrations e.g. for use in multi-tenant environments.
- The way checksums are calculated has changed, since migration files are just ordinary rust code running rustfmt could change the checksum of migrations even if the queries were unchanged. Currently only the executed or prepared SQL queries are taken into account which will not change with formatting. The checksums for pure SQL migrations is unchanged.

### Other

- The library now uses unsafe to mask lifetimes so that the migration functions could be ergonomic. The unsafe blocks are all checked and documented, unfortunately testing with miri is not yet possible.
- `dotenv` dependency was replaced with `dotenvy`
- The `validate-sql` feature has been removed.

## 0.5.0

### Other

- Bumped sqlx to `0.6.0`.

## 0.4.0

### Features

- Added Sqlite support

## 0.3.2

### Fixes

- Removed `sqlx/runtime-tokio-rustls` CLI feature dependency

## 0.3.1

### Misc

- Relaxed dependency version requirements

## 0.3.0

### Features

- Support loading `.env` in the CLI.

### Misc

- Show feature flags in rustdoc

## 0.2.1

### Fixes

- fixed crates.io categories

## 0.2.0

### Features

- Ability to log SQL queries in the CLI
- Ability to check/validate migrations in the CLI
- Optional SQL validation during code generation

### Misc

- Added "revertible" as a more correct alias where the term "reversible" is used

## 0.1.0 - 0.1.2

Initial versions
