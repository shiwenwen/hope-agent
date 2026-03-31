fn main() {
    // Enable SQLite FTS5 full-text search for the memory system.
    // rusqlite's bundled SQLite build picks up SQLITE_ENABLE_FTS5 via this env var.
    std::env::set_var("LIBSQLITE3_FLAGS", "-DSQLITE_ENABLE_FTS5");

    // Link macOS frameworks for permission checking
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
    }

    tauri_build::build()
}
