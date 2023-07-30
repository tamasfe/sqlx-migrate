use crate::DatabaseType;
use proc_macro2::{Ident, Span};
use std::{fs, path::Path};
use syn::parse_quote;
use walkdir::WalkDir;

/// Generate Rust code from a migrations directory.
/// It is meant to be used in `build.rs`.
///
/// # Panics
///
/// This function is meant to be used in `build.rs` and will panic on errors.
pub fn generate(
    migrations_dir: impl AsRef<Path>,
    module_path: impl AsRef<Path>,
    db_type: DatabaseType,
) {
    cargo_rerun(migrations_dir.as_ref());

    let modules = super::migration_modules(migrations_dir.as_ref());
    let migrations = super::migrations(db_type, migrations_dir.as_ref());

    if let Some(p) = module_path.as_ref().parent() {
        fs::create_dir_all(p).unwrap();
    }

    let db_ident = Ident::new(db_type.sqlx_type(), Span::call_site());

    fs::write(
        module_path,
        prettyplease::unparse(&parse_quote! {
            pub use sqlx_migrate::prelude::*;

            #modules

            /// All the migrations.
            pub fn migrations() -> impl IntoIterator<Item = Migration<sqlx::#db_ident>> {
                #migrations
            }

        }),
    )
    .unwrap();
}

fn cargo_rerun(dir: &Path) {
    for entry in WalkDir::new(dir) {
        let Ok(entry) = entry else { continue };
        println!("cargo:rerun-if-changed={}", entry.path().to_string_lossy());
    }
}
