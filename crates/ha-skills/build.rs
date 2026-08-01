fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // rust-embed（`skills::embedded`）enumerates the embedded tree at macro
    // expansion; existing files land in dep-info via include_bytes!, but
    // ADDED/REMOVED files are invisible to cargo's fingerprint without this
    // (a warm-target release rebuild would silently ship the previous file
    // set). 阶段 5 第七刀随 `embedded.rs` 自 ha-core/build.rs 迁来——
    // `crates/ha-skills/` 与 `crates/ha-core/` 同深度，相对路径不变。
    println!("cargo:rerun-if-changed=../../skills");
}
