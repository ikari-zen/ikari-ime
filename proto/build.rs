//! protox（純 Rust の protobuf コンパイラ）で proto をコンパイルし、prost でコードを生成する。
//! protoc は要らない（ADR 0004 §5.2）。生成したコードは OUT_DIR に置き、コミットしない。

use std::error::Error;

/// engine.proto（エンジンと殻。ADR 0004 §5.2）と shell.proto（殻の入口と shell-core。ADR 0041 §4.4）。
/// bridge.proto は Swift だけが使う（macos/scripts/gen-proto.sh）。
const PROTO_FILES: [&str; 2] = ["ikari/engine/v1/engine.proto", "ikari/shell/v1/shell.proto"];
const PROTO_ROOT: &str = ".";

fn main() -> Result<(), Box<dyn Error>> {
    for file in PROTO_FILES {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile(PROTO_FILES, [PROTO_ROOT])?;
    prost_build::Config::new().compile_fds(fds)?;
    Ok(())
}
