fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // rust-embed enumerates the embedded tree at macro expansion; existing
    // files land in dep-info via include_bytes!, but ADDED/REMOVED files are
    // invisible to cargo's fingerprint without this (a warm-target release
    // rebuild would silently ship the previous file set).
    //
    // 只剩手册（`manual/`）——`skills/` 那行已随 `skills::embedded` 迁进
    // `crates/ha-skills/build.rs`，两个 crate 各自守自己的嵌入树。
    println!("cargo:rerun-if-changed=../../docs/user-guide");

    // protoc / prost 只服务飞书长连接的 pbbp2 帧，已随 ha-channel 迁出——
    // ha-core 因此甩掉 `prost-build` + `protoc-bin-vendored` 两个构建依赖。
}
