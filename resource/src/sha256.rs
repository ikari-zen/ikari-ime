//! ストリームでの sha256。大きいファイル（辞書で数百 MB）でもメモリは読み取りバッファの分だけ。
//! 340MB 級の所要時間は S0（#254）で測る。

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

const READ_BUFFER_BYTES: usize = 8 * 1024;
pub(crate) const SHA256_HEX_LEN: usize = 64;

/// 小文字 16 進 64 文字。
pub fn sha256_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; READ_BUFFER_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(SHA256_HEX_LEN);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    sha256_reader(File::open(path)?)
}

/// manifest の sha256 の欄の形（小文字 16 進 64 文字）
pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == SHA256_HEX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_reader(&b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_reader(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn spans_buffer_boundaries() {
        // 8KB のバッファをまたぐ長さでも、一括と同じ値になる
        let data = vec![0x5au8; READ_BUFFER_BYTES * 3 + 17];
        let streamed = sha256_reader(&data[..]).unwrap();
        let whole = format!("{:x}", Sha256::digest(&data));
        assert_eq!(streamed, whole);
    }
}
