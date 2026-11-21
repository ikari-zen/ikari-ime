//! スパイク（#486。ADR 0074 §8。捨てるコード。main には入れない）。
//!
//! llama.cpp の abort callback（`llama_set_abort_callback`）が、CPU と GPU（Vulkan）の decode で
//! 呼ばれるか、`true` を返したら decode が止まるかを、1 回の実行で比べる。
//!
//! 使い方: `abort-probe <model.gguf> [cpu|gpu|both]`（既定 both）。環境変数 `IKARI_LM_GPU` は見ない。
//! Vulkan のビルドでは、exe の隣の `backends\` からバックエンドを読む（`probe_devices` と同じ）。
//!
//! 各装置で 3 回 decode する（同じ長いプロンプト）。出力は標準エラーに `ABORTPROBE ...` の行:
//! - `count`: callback は常に `false`（止めない）を返し、呼ばれた回数と decode の時間と戻り値を出す
//! - `abort_first`: callback は最初から `true` を返す。decode が早く戻るか、戻り値（2 = 中止）を出す
//! - `abort_after_2ms`: 別スレッドが 2 ms 後に印を立て、callback が印を見て `true` を返す。decode の時間と戻り値

use std::ffi::{CString, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use llama_cpp_sys_2 as sys;

/// プロンプトの繰り返し回数。zenz は位置埋め込みが 256 までなので、トークン数をそれ未満にする（1 文字 = 1 トークン）
const PROMPT_REPEATS: usize = 8;
const PROMPT_UNIT: &str = "これはテストの文章です。日本語の入力を変換します。";
const N_GPU_LAYERS_ALL: i32 = 999;
const ABORT_AFTER: Duration = Duration::from_millis(2);

struct State {
    calls: AtomicU64,
    /// true を返すか
    abort: AtomicBool,
}

unsafe extern "C" fn on_abort(data: *mut c_void) -> bool {
    // SAFETY: `data` は `run` の `State` へのポインタ。decode の間、`run` が生きている
    let st = unsafe { &*(data as *const State) };
    st.calls.fetch_add(1, Ordering::Relaxed);
    st.abort.load(Ordering::Relaxed)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(model_path) = args.next() else {
        eprintln!("usage: abort-probe <model.gguf> [cpu|gpu|both]");
        std::process::exit(2);
    };
    let which = args.next().unwrap_or_else(|| "both".to_owned());
    // バックエンドの初期化と、`backends\` の読み込み
    let devices = ikari_lm::probe_devices().unwrap_or_else(|e| {
        eprintln!("backend init failed: {e}");
        std::process::exit(1);
    });
    eprintln!("ABORTPROBE devices={devices}");
    let has_gpu = devices.split(',').any(|d| d != "cpu");
    if which != "gpu" {
        run_device(&model_path, false);
    }
    if which != "cpu" {
        if has_gpu {
            run_device(&model_path, true);
        } else {
            eprintln!("ABORTPROBE gpu skipped: no GPU device registered");
        }
    }
}

fn run_device(model_path: &str, gpu: bool) {
    let label = if gpu { "gpu" } else { "cpu" };
    let path = CString::new(model_path).expect("path has no NUL");
    // SAFETY: llama.cpp の C API。ポインタは、この関数の中で作って、この関数の中で解放する
    unsafe {
        let mut mp = sys::llama_model_default_params();
        // CPU: 空のデバイス一覧（NULL だけ）を渡す。GPU: 既定の選択に任せて全層を載せる
        let mut cpu_only_devices: [sys::ggml_backend_dev_t; 1] = [std::ptr::null_mut()];
        if gpu {
            mp.n_gpu_layers = N_GPU_LAYERS_ALL;
        } else {
            mp.devices = cpu_only_devices.as_mut_ptr();
            mp.n_gpu_layers = 0;
        }
        // zenz の GGUF は llama.cpp が知らない `tokenizer.ggml.pre` を持つので、`default` に上書きする（`llama.rs` と同じ）。
        // 配列は、キーが空の要素で終わる
        let mut overrides: [sys::llama_model_kv_override; 2] = std::mem::zeroed();
        overrides[0].tag = sys::LLAMA_KV_OVERRIDE_TYPE_STR;
        for (dst, src) in overrides[0].key.iter_mut().zip(b"tokenizer.ggml.pre") {
            *dst = *src as std::os::raw::c_char;
        }
        for (dst, src) in overrides[0]
            .__bindgen_anon_1
            .val_str
            .iter_mut()
            .zip(b"default")
        {
            *dst = *src as std::os::raw::c_char;
        }
        mp.kv_overrides = overrides.as_ptr();
        let model = sys::llama_model_load_from_file(path.as_ptr(), mp);
        if model.is_null() {
            eprintln!("ABORTPROBE {label} model load failed");
            return;
        }
        let vocab = sys::llama_model_get_vocab(model);
        let text = PROMPT_UNIT.repeat(PROMPT_REPEATS);
        let mut tokens = vec![0 as sys::llama_token; text.len() + 8];
        let n = sys::llama_tokenize(
            vocab,
            text.as_ptr().cast(),
            i32::try_from(text.len()).unwrap_or(i32::MAX),
            tokens.as_mut_ptr(),
            i32::try_from(tokens.len()).unwrap_or(i32::MAX),
            true,
            true,
        );
        if n <= 0 {
            eprintln!("ABORTPROBE {label} tokenize failed: {n}");
            sys::llama_model_free(model);
            return;
        }
        tokens.truncate(usize::try_from(n).unwrap_or(0));
        eprintln!(
            "ABORTPROBE {label} prompt_tokens={n} min={:?} max={:?} n_vocab={}",
            tokens.iter().min(),
            tokens.iter().max(),
            sys::llama_vocab_n_tokens(vocab)
        );

        for mode in ["count", "abort_first", "abort_after_2ms", "count"] {
            let mut cp = sys::llama_context_default_params();
            let n_ctx = u32::try_from(n).unwrap_or(0) + 64;
            cp.n_ctx = n_ctx;
            cp.n_batch = n_ctx;
            cp.n_ubatch = n_ctx;
            let ctx = sys::llama_init_from_model(model, cp);
            if ctx.is_null() {
                eprintln!("ABORTPROBE {label} {mode} context failed");
                continue;
            }
            let state = State {
                calls: AtomicU64::new(0),
                abort: AtomicBool::new(mode == "abort_first"),
            };
            sys::llama_set_abort_callback(
                ctx,
                Some(on_abort),
                std::ptr::from_ref(&state).cast_mut().cast(),
            );
            let t0 = Instant::now();
            let rc = std::thread::scope(|scope| {
                if mode == "abort_after_2ms" {
                    scope.spawn(|| {
                        std::thread::sleep(ABORT_AFTER);
                        state.abort.store(true, Ordering::Relaxed);
                    });
                }
                let batch = sys::llama_batch_get_one(tokens.as_mut_ptr(), n);
                let rc = sys::llama_decode(ctx, batch);
                // GPU は非同期のことがある。logits を待つ同期まで含めた時間も見る
                let t_decode = t0.elapsed();
                sys::llama_synchronize(ctx);
                (rc, t_decode)
            });
            eprintln!(
                "ABORTPROBE {label} mode={mode} rc={} decode_ms={:.2} total_ms={:.2} callback_calls={}",
                rc.0,
                rc.1.as_secs_f64() * 1000.0,
                t0.elapsed().as_secs_f64() * 1000.0,
                state.calls.load(Ordering::Relaxed)
            );
            // コールバックを外してから解放する（`state` はこのループの終わりで落ちる）
            sys::llama_set_abort_callback(ctx, None, std::ptr::null_mut());
            sys::llama_free(ctx);
        }
        sys::llama_model_free(model);
    }
}
