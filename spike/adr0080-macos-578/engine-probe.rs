//! スパイク #578（ADR 0080 C1）。捨てるコード。マージしない。
//!
//! App Sandbox の中で、自分の束（`<束>/Contents/Resources/data/`）の辞書とモデルを、
//! 本物の `Engine::open` で開き、かかった時間を測る。辞書は、コンテナの中に作った試験用の
//! データの場所の `dict/` を、束の `dict/` へのシンボリックリンクにして見せる
//! （Sandbox はリンクを辿った先のパスで判定する。今のエンジンは束を直接は見ないため）。
//!
//! 使い方（束の中の `Contents/MacOS/engine-probe` として置き、署名してから）:
//!   engine-probe engine          Engine::open（LM なし）と、モデルの sha256 の時間
//!   engine-probe mmap            4 ファイルを mmap し、mincore で常駐の割合を見てから全ページに触る
//!   engine-probe deny <path>     束の外のファイルが読めないこと（Sandbox が効いていることの対照）
use std::ffi::c_void;
use std::fs::{self, File};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikari_engine::{Engine, EngineConfig};

const FILES: [&str; 4] = [
    "dict/system.dict",
    "dict/grammar.bin",
    "dict/vectors.bin",
    "models/zenz-v3.2-small-Q5_K_M.gguf",
];
const MODEL: &str = "models/zenz-v3.2-small-Q5_K_M.gguf";
const SYSTEM_DICT: &str = "dict/system.dict";
const TEST_DATA_DIR: &str = "spike578-data";
const IDLE_UNLOAD_SECS: u32 = 300;

unsafe extern "C" {
    fn sandbox_check(pid: libc::pid_t, operation: *const libc::c_char, kind: libc::c_int, ...) -> libc::c_int;
    fn mincore(addr: *const c_void, len: libc::size_t, vec: *mut libc::c_char) -> libc::c_int;
}

fn page_size() -> usize {
    // SAFETY: sysconf は引数だけを読む
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

fn data_root() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    // <束>/Contents/MacOS/engine-probe → <束>/Contents/Resources/data
    exe.parent().and_then(Path::parent).expect("Contents").join("Resources/data")
}

/// mmap したもの。drop で munmap する
struct Map {
    ptr: *mut c_void,
    len: usize,
    _file: File,
}

impl Map {
    fn open(path: &Path) -> std::io::Result<Map> {
        let file = File::open(path)?;
        let len = file.metadata()?.len() as usize;
        // SAFETY: 読むだけの私的な写像。len はファイルの大きさ
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0)
        };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Map { ptr, len, _file: file })
    }

    /// 常駐しているページの割合（%）
    fn resident_percent(&self) -> f64 {
        let ps = page_size();
        let pages = self.len.div_ceil(ps);
        let mut vec = vec![0 as libc::c_char; pages];
        // SAFETY: vec はページ数ぶん。ptr..ptr+len は写像の中
        let rc = unsafe { mincore(self.ptr, self.len, vec.as_mut_ptr()) };
        if rc != 0 {
            return -1.0;
        }
        let n = vec.iter().filter(|&&b| b & 1 != 0).count();
        n as f64 * 100.0 / pages as f64
    }

    /// 全ページの 1 バイトを読む
    fn touch(&self) -> u64 {
        let ps = page_size();
        let mut sum = 0u64;
        let mut off = 0;
        while off < self.len {
            // SAFETY: off < len
            sum = sum.wrapping_add(unsafe { std::ptr::read_volatile((self.ptr as *const u8).add(off)) } as u64);
            off += ps;
        }
        sum
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: open で得た写像
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn header() {
    // SAFETY: 操作名なしの問い合わせ（Sandbox の中なら 1）
    let sandboxed = unsafe { sandbox_check(libc::getpid(), std::ptr::null(), 0) };
    println!("sandboxed={sandboxed} HOME={}", std::env::var("HOME").unwrap_or_default());
    println!("data_root={}", data_root().display());
}

fn run_mmap() {
    let root = data_root();
    let all = Instant::now();
    let mut total = 0usize;
    for name in FILES {
        let path = root.join(name);
        let t = Instant::now();
        let map = match Map::open(&path) {
            Ok(m) => m,
            Err(e) => {
                println!("{name}: open/mmap FAILED: {e}");
                continue;
            }
        };
        let t_map = ms(t);
        let before = map.resident_percent();
        let t2 = Instant::now();
        let sum = map.touch();
        let t_touch = ms(t2);
        let after = map.resident_percent();
        total += map.len;
        println!(
            "{name}: size={:.1}MB mmap={t_map:.2}ms resident_before={before:.1}% touch_all={t_touch:.1}ms resident_after={after:.1}% (sum={sum})",
            map.len as f64 / 1e6
        );
    }
    println!("mmap total: {:.1}MB in {:.1}ms", total as f64 / 1e6, ms(all));
}

fn run_engine() {
    let root = data_root();
    // 開く前の常駐（mincore はページを読み込まない）
    for name in [SYSTEM_DICT, MODEL] {
        match Map::open(&root.join(name)) {
            Ok(m) => println!("{name}: resident_before={:.1}%", m.resident_percent()),
            Err(e) => println!("{name}: mmap FAILED: {e}"),
        }
    }
    let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
    let dir = home.join(TEST_DATA_DIR);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test data dir");
    std::os::unix::fs::symlink(root.join("dict"), dir.join("dict")).expect("symlink dict");
    let t = Instant::now();
    let opened = Engine::open(&EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: IDLE_UNLOAD_SECS,
        ..Default::default()
    });
    let t_open = ms(t);
    match &opened {
        Ok(_) => println!("Engine::open OK in {t_open:.1}ms (LM なし)"),
        Err(e) => println!("Engine::open FAILED in {t_open:.1}ms: {e:?}"),
    }
    if let Ok(m) = Map::open(&root.join(SYSTEM_DICT)) {
        println!("{SYSTEM_DICT}: resident_after_open={:.1}%", m.resident_percent());
    }
    drop(opened);
    // モデルの照合（resolve_preset が LM を求めるときに数える sha256 と同じ読み方）
    let t = Instant::now();
    match ikari_resource::sha256_file(&root.join(MODEL)) {
        Ok(h) => println!("model sha256 {:.16}… in {:.1}ms", h, ms(t)),
        Err(e) => println!("model sha256 FAILED: {e}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

fn run_deny(path: &str) {
    match fs::read(path) {
        Ok(b) => println!("deny-check: READ OK ({} bytes) — Sandbox が効いていない", b.len()),
        Err(e) => println!("deny-check: {path}: {e} (raw={:?})", e.raw_os_error()),
    }
}

fn main() {
    header();
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("mmap") => run_mmap(),
        Some("engine") => run_engine(),
        Some("deny") => run_deny(args.get(2).map(String::as_str).unwrap_or("/etc/hosts")),
        _ => eprintln!("usage: engine-probe engine|mmap|deny <path>"),
    }
}
