fn main() {
    // Enable SQLite FTS5 full-text search for the memory system.
    // rusqlite's bundled SQLite build picks up SQLITE_ENABLE_FTS5 via this env var.
    std::env::set_var("LIBSQLITE3_FLAGS", "-DSQLITE_ENABLE_FTS5");

    // Immutable build identity used by local diagnostic evaluation plans. A
    // packaged build without git metadata remains dirty/fail-closed and can
    // never be mistaken for protected exact-SHA evidence.
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| "0000000000000000000000000000000000000000".to_string());
    let dirty = std::process::Command::new("git")
        .current_dir("..")
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)src-tauri/binaries/hope-agent-eval-*",
        ])
        .output()
        .map_or(true, |output| {
            !output.status.success() || !output.stdout.is_empty()
        });
    println!("cargo:rustc-env=HA_BUILD_COMMIT_SHA={commit}");
    println!(
        "cargo:rustc-env=HA_BUILD_GIT_DIRTY={}",
        if dirty { "1" } else { "0" }
    );
    for git_path in ["HEAD", "index"] {
        if let Some(path) = std::process::Command::new("git")
            .args(["rev-parse", "--git-path", git_path])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    // Link macOS frameworks for permission checking
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
    }

    tauri_build::build()
}
