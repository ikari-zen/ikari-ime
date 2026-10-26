//! protox（純 Rust の protobuf コンパイラ）で proto をコンパイルし、prost でコードを生成する。
//! protoc は要らない（ADR 0004 §5.2）。生成したコードは OUT_DIR に置き、コミットしない。

use std::error::Error;

const PROTO_FILE: &str = "ikari/engine/v1/engine.proto";
const PROTO_ROOT: &str = ".";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={PROTO_FILE}");
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile([PROTO_FILE], [PROTO_ROOT])?;
    prost_build::Config::new().compile_fds(fds)?;
    Ok(())
}
