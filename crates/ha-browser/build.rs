fn main() {
    // rust-embed 的 Chrome 扩展文件树（browser/extension/embedded.rs）——
    // warm-target release 下漏了这行会静默 ship 旧扩展文件集（接线清单 §8，
    // 自 ha-core build.rs 随 embed 模块迁入）。
    println!("cargo:rerun-if-changed=../../extensions/chrome");
}
