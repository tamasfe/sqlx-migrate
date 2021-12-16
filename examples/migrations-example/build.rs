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
