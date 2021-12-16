use std::path::Path;

fn main() {
    sqlx_migrate::cli::run(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
        migrations_example::generated::migrations(),
    );
}
