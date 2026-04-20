//! Pre-compile hook: ensure `dist/index.html` exists before `rust-embed`
//! scans the folder, so a fresh `cargo build` on a clone without a prior
//! `npm run build` still produces a working binary.
//!
//! When the real Vite output already exists we leave it alone — this
//! script only fills in placeholders when the directory is empty.

use std::fs;
use std::path::PathBuf;

const PLACEHOLDER_INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Hope Agent — Front-end not built</title>
  <link rel="stylesheet" href="/placeholder.css" />
</head>
<body>
  <main>
    <h1>Hope Agent is running</h1>
    <p>
      The Web GUI has not been built yet. Run
      <code>npm run build</code> in the project root and restart
      <code>hope-agent server</code> to load the real interface.
    </p>
    <p>The backend API remains fully functional at <code>/api/*</code>.</p>
  </main>
</body>
</html>
"#;

const PLACEHOLDER_CSS: &str = r#"html,body{margin:0;padding:0;font-family:system-ui,-apple-system,sans-serif;
background:#0b0d11;color:#e6e6e6;min-height:100vh;display:flex;
align-items:center;justify-content:center}
main{max-width:560px;padding:2rem;border:1px solid #2a2d33;border-radius:12px;background:#14171c}
h1{font-size:1.6rem;margin:0 0 1rem 0}
p{line-height:1.6;margin:0 0 1rem 0}
code{background:#1f232a;padding:.15rem .35rem;border-radius:4px;font-size:.9em}"#;

fn main() {
    // Resolve ../../dist relative to this crate.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist = manifest.join("..").join("..").join("dist");

    if let Err(e) = fs::create_dir_all(&dist) {
        println!("cargo:warning=ha-server build.rs: failed to create {}: {}", dist.display(), e);
        return;
    }

    let index = dist.join("index.html");
    if !index.exists() {
        if let Err(e) = fs::write(&index, PLACEHOLDER_INDEX_HTML) {
            println!(
                "cargo:warning=ha-server build.rs: failed to write placeholder index.html: {}",
                e
            );
        }
    }

    let css = dist.join("placeholder.css");
    if !css.exists() {
        if let Err(e) = fs::write(&css, PLACEHOLDER_CSS) {
            println!(
                "cargo:warning=ha-server build.rs: failed to write placeholder.css: {}",
                e
            );
        }
    }

    // Rerun when the dist folder changes so a `npm run build` that
    // refreshes assets triggers a rebuild of the embed bundle.
    println!("cargo:rerun-if-changed={}", dist.display());
}
