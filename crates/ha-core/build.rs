use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto/pbbp2.proto");
    println!("cargo:rerun-if-env-changed=PROTOC");

    if env::var_os("PROTOC").is_none() {
        env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    }

    prost_build::compile_protos(&["proto/pbbp2.proto"], &["proto"])
        .expect("compile pbbp2.proto");
}
