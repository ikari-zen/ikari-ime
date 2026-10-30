//! protox（純 Rust の protobuf コンパイラ）で proto をコンパイルし、prost でコードを生成する。
//! protoc は要らない（ADR 0004 §5.2）。生成したコードは OUT_DIR に置き、コミットしない。

use std::error::Error;

/// engine.proto（エンジンと殻。ADR 0004 §5.2）、shell.proto（殻の入口と shell-core。ADR 0041 §4.4）、
/// bridge.proto（設定アプリと殻。ADR 0005 §6.3。Swift は macos/scripts/gen-proto.sh で生成する）、
/// settings.proto（設定アプリと shell-core。ADR 0041 §2.3）。
const PROTO_FILES: [&str; 4] = [
    "ikari/engine/v1/engine.proto",
    "ikari/shell/v1/shell.proto",
    "ikari/bridge/v1/bridge.proto",
    "ikari/settings/v1/settings.proto",
];
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
