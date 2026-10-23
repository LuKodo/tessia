//! Re-embeds the Diesel migrations whenever the `migrations/` tree changes
//! (a new migration directory would otherwise not trigger a rebuild) and the
//! embedded desktop UI whenever `desktop-ui/dist` changes.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
    // The desktop window embeds the UI build; new or removed files must re-embed.
    println!("cargo:rerun-if-changed=desktop-ui/dist");
}
