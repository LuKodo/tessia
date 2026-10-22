//! Re-embeds the Diesel migrations whenever the `migrations/` tree changes
//! (a new migration directory would otherwise not trigger a rebuild).

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
