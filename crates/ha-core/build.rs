use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto/pbbp2.proto");
    // rust-embed enumerates the embedded trees at macro expansion; existing
    // files land in dep-info via include_bytes!, but ADDED/REMOVED files are
    // invisible to cargo's fingerprint without these (a warm-target release
    // rebuild would silently ship the previous file set).
    println!("cargo:rerun-if-changed=../../skills");
    println!("cargo:rerun-if-changed=../../extensions/chrome");
    println!("cargo:rerun-if-changed=../../docs/user-guide");
    println!("cargo:rerun-if-env-changed=PROTOC");

    if env::var_os("PROTOC").is_none() {
        env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    }

    prost_build::compile_protos(&["proto/pbbp2.proto"], &["proto"]).expect("compile pbbp2.proto");
}
