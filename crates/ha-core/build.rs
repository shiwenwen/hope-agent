fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // rust-embed enumerates the embedded trees at macro expansion; existing
    // files land in dep-info via include_bytes!, but ADDED/REMOVED files are
    // invisible to cargo's fingerprint without these (a warm-target release
    // rebuild would silently ship the previous file set).
    println!("cargo:rerun-if-changed=../../skills");
    println!("cargo:rerun-if-changed=../../docs/user-guide");

    // protoc / prost 只服务飞书长连接的 pbbp2 帧，已随 ha-channel 迁出——
    // ha-core 因此甩掉 `prost-build` + `protoc-bin-vendored` 两个构建依赖。
}
