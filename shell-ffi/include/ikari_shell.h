// ikari-shell-ffi の C ABI（ADR 0004 §4.3、ADR 0041 §4.4。契約）。殻（IME のプロセス）が使う。
// - ikari_composer_*、ikari_convert_script: ローマ字・かな入力と字種変換（ADR 0004。L-E の入れ替えで消す）
// - ikari_shell_*、ikari_settings_*: 殻の OS に依存しない処理（shell-core）への 1 本の口（ADR 0041）
// 辞書や llama.cpp には依存しない。Rust の panic は FFI の外へ出ない。
#ifndef IKARI_SHELL_H
#define IKARI_SHELL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct IkariComposer IkariComposer;

// 借用。同じ composer を次に呼ぶまで有効（UTF-8、NUL 終端なし）。
typedef struct {
    const uint8_t *ptr;
    size_t len;
} IkariStr;

// Rust が確保したバイト列。ikari_shell_buf_free で1回だけ解放する。
// （ADR 0004 §4.3 の IkariBuf / ikari_buf_free は、エンジンの FFI と同じ名前で衝突するため改名した）
typedef struct {
    uint8_t *ptr;
    size_t len;
} IkariShellBuf;

// jis_key の番号（Rust の ikari_input::JisKey の宣言順。変更しない）。
typedef enum {
    IKARI_JIS_DIGIT1 = 0,
    IKARI_JIS_DIGIT2 = 1,
    IKARI_JIS_DIGIT3 = 2,
    IKARI_JIS_DIGIT4 = 3,
    IKARI_JIS_DIGIT5 = 4,
    IKARI_JIS_DIGIT6 = 5,
    IKARI_JIS_DIGIT7 = 6,
    IKARI_JIS_DIGIT8 = 7,
    IKARI_JIS_DIGIT9 = 8,
    IKARI_JIS_DIGIT0 = 9,
    IKARI_JIS_MINUS = 10,
    IKARI_JIS_CARET = 11,
    IKARI_JIS_YEN = 12,
    IKARI_JIS_AT = 13,
    IKARI_JIS_BRACKET_LEFT = 14,
    IKARI_JIS_SEMICOLON = 15,
    IKARI_JIS_COLON = 16,
    IKARI_JIS_BRACKET_RIGHT = 17,
    IKARI_JIS_COMMA = 18,
    IKARI_JIS_PERIOD = 19,
    IKARI_JIS_SLASH = 20,
    IKARI_JIS_BACKSLASH = 21,  // ろ
    IKARI_JIS_A = 22,
    IKARI_JIS_B = 23,
    IKARI_JIS_C = 24,
    IKARI_JIS_D = 25,
    IKARI_JIS_E = 26,
    IKARI_JIS_F = 27,
    IKARI_JIS_G = 28,
    IKARI_JIS_H = 29,
    IKARI_JIS_I = 30,
    IKARI_JIS_J = 31,
    IKARI_JIS_K = 32,
    IKARI_JIS_L = 33,
    IKARI_JIS_M = 34,
    IKARI_JIS_N = 35,
    IKARI_JIS_O = 36,
    IKARI_JIS_P = 37,
    IKARI_JIS_Q = 38,
    IKARI_JIS_R = 39,
    IKARI_JIS_S = 40,
    IKARI_JIS_T = 41,
    IKARI_JIS_U = 42,
    IKARI_JIS_V = 43,
    IKARI_JIS_W = 44,
    IKARI_JIS_X = 45,
    IKARI_JIS_Y = 46,
    IKARI_JIS_Z = 47,
    IKARI_JIS_KEY_COUNT = 48,
} IkariJisKey;

// convert_script の字種（Rust の ikari_input::Script の値と一致）。
typedef enum {
    IKARI_SCRIPT_HIRAGANA = 0,            // F6
    IKARI_SCRIPT_FULLWIDTH_KATAKANA = 1,  // F7
    IKARI_SCRIPT_HALFWIDTH_KATAKANA = 2,  // F8
    IKARI_SCRIPT_FULLWIDTH_ALNUM = 3,     // F9
    IKARI_SCRIPT_HALFWIDTH_ALNUM = 4,     // F10
} IkariScript;

// table_tsv: ローマ字表（Mozc 互換の TSV）。len == 0 なら同梱の表。
// 失敗なら NULL を返し、*err にエラー文（UTF-8、ユーザーの入力は含まない）を入れる。err は NULL でもよい。
IkariComposer *ikari_composer_new_romaji(const uint8_t *table_tsv, size_t len, IkariShellBuf *err);
IkariComposer *ikari_composer_new_jis(void);

void ikari_composer_push_char(IkariComposer *c, uint32_t codepoint);          // ローマ字
void ikari_composer_push_jis(IkariComposer *c, uint8_t jis_key, bool shift);  // IkariJisKey。範囲外は無視
bool ikari_composer_backspace(IkariComposer *c);                              // 消せたら true（pending から先に消す）
void ikari_composer_flush(IkariComposer *c);                                  // pending の「n」などを確定する
void ikari_composer_clear(IkariComposer *c);
IkariStr ikari_composer_kana(const IkariComposer *c);
IkariStr ikari_composer_pending(const IkariComposer *c);                      // JIS かなでは常に空
void ikari_composer_free(IkariComposer *c);                                   // NULL は何もしない

// F6〜F10。script が範囲外なら入力をそのまま返す。
IkariShellBuf ikari_convert_script(const uint8_t *s, size_t len, uint32_t script);
// ptr が NULL なら何もしない。同じ buf を 2 回渡さない（値渡しなので Rust 側では検出できない）。
void ikari_shell_buf_free(IkariShellBuf buf);

// ================================================================ 殻の 1 本の口（ADR 0041 §4.4。契約 M1-b）
//
// 符号化: イベント・動作・ホストの返事・設定は protobuf（proto/ikari/shell/v1/shell.proto。ADR 0041 §4.4、
//   M0-a の計測で決定）。文字列は UTF-8、位置と長さは UTF-16 の単位。
// 所有権:
//   - IkariStr の引数は借用。呼び出しの間だけ読む（返ったあとは参照しない）。
//   - Rust が返す IkariShellBuf（*out、*err、*actions）は呼び出し側のもの。ikari_shell_buf_free で 1 回だけ解放する。
//   - ホストのコールバックが *out に入れる IkariShellBuf は、ikari_shell_buf_copy で作ったものに限る。
//     入れた時点で Rust のものになり、Rust が解放する（コールバックの戻り値に関係なく）。
// スレッド:
//   - ikari_shell_new / dispatch / free は 1 つのスレッド（macOS は主スレッド）から呼ぶ。
//   - ikari_shell_engine_down だけは、どのスレッドからでも、いつでも呼べる（境界の唯一の例外。§4.3）。
//   - ikari_settings_*、ikari_shell_buf_copy、ikari_shell_buf_free はどのスレッドからでも呼べる（状態を持たない）。
//   - コールバックの中から、同じ shell の ikari_shell_dispatch を呼び直さない（IKARI_SHELL_ERR_BUSY になる）。
// panic: Rust の panic は FFI の外へ出ない。IKARI_SHELL_ERR_PANIC が返ったら、その shell を free して作り直す。

// 不透明なハンドル。指す先のない番号で、C 側は値を持つだけにする。free のあとの使用と二重の free は
// Rust が検出する（番号を使い回さないので、別の shell を誤って解放することもない）。
typedef struct IkariShell IkariShell;

// ikari_shell_* と ikari_settings_* の戻り値。
typedef enum {
    IKARI_SHELL_OK = 0,
    IKARI_SHELL_ERR_NULL = 1,            // 必須の引数（出力先など）が NULL
    IKARI_SHELL_ERR_INVALID_HANDLE = 2,  // NULL、解放ずみ、知らない shell
    IKARI_SHELL_ERR_DECODE = 3,          // 入力のバイト列が protobuf として読めない、型の範囲外、UTF-8 でない
    IKARI_SHELL_ERR_BUSY = 4,            // 同じ shell の dispatch の途中（コールバックの中からの呼び直しなど）
    IKARI_SHELL_ERR_PANIC = 5,           // Rust の中で panic した。shell を作り直す
    IKARI_SHELL_ERR_UNIMPLEMENTED = 6,   // まだ実装していない（契約の stub）
} IkariShellStatus;

// ホストの engine_request の戻り値（トランスポートの誤り。中身の誤りは Rust が返事を読んで決める。§4.3）。
// これ以外の値は UNAVAILABLE として扱う。
typedef enum {
    IKARI_SHELL_TRANSPORT_OK = 0,           // *reply に返事を入れた
    IKARI_SHELL_TRANSPORT_UNAVAILABLE = 1,  // つながらない、切れた
    IKARI_SHELL_TRANSPORT_TIMEOUT = 2,      // timeout_ms を超えた
} IkariShellTransportResult;

// ikari_shell_engine_down の kind（XPC の interruptionHandler / invalidationHandler）。範囲外は無視する。
typedef enum {
    IKARI_SHELL_DOWN_INTERRUPTED = 0,
    IKARI_SHELL_DOWN_INVALIDATED = 1,
} IkariShellDownKind;

// ikari_shell_new の設定（借用。ikari_shell_new の間だけ読む）。
typedef struct {
    IkariStr shell_version;  // 殻の版（エンジンとの hello の照合）
    IkariStr settings_path;  // settings.tsv の絶対パス。空なら既定値だけで動かす（テスト用）
    IkariStr resources_dir;  // 同梱の資源の置き場所（.app の Resources）
} IkariShellConfig;

// ホストの口（§4.3、§4.4）。ctx はすべてのコールバックの第 1 引数にそのまま渡る。
// release 以外は NULL にしない（NULL があると ikari_shell_new が失敗する）。
//
// - surrounding / selection: ikari_shell_dispatch の中から、dispatch を呼んだスレッドで同期に呼ぶ（要るときだけ）。
//   取れたら *out に SurroundingText / SelectedText の protobuf を入れて 0 を返す。取れなければ 0 以外を返す。
// - engine_*: どのスレッドからでも、並行に呼ばれうる（設定アプリの中継が返事を待つ間も主スレッドが変換する）。
//   ctx はそれに耐えること。
//   - engine_connect: 新しい接続を作り、0 以外の番号を返す。番号は接続ごとに替える（再接続の世代を兼ねる）。
//     つながらなければ 0。
//   - engine_request: bytes（engine.proto の Envelope）を送り、返事を同期に待つ。返事は *reply に入れる。
//     戻り値は IkariShellTransportResult。
//   - engine_post: 返事を待たずに送る。engine_close: 接続を閉じる。
//   - engine_pid: 相手のプロセスの番号。分からなければ 0 以下。
// - release: Rust が ctx を使い終えたときに 1 回だけ呼ぶ（NULL なら呼ばない）。ikari_shell_free より後になることが
//   ある（別のスレッドの engine_down が終わるまで待つ）。ikari_shell_new が失敗したときも呼ぶ。
typedef struct {
    void *ctx;
    int32_t (*surrounding)(void *ctx, IkariShellBuf *out);
    int32_t (*selection)(void *ctx, IkariShellBuf *out);
    uint64_t (*engine_connect)(void *ctx);
    int32_t (*engine_request)(void *ctx, uint64_t handle, IkariStr bytes, uint32_t timeout_ms, IkariShellBuf *reply);
    void (*engine_post)(void *ctx, uint64_t handle, IkariStr bytes);
    void (*engine_close)(void *ctx, uint64_t handle);
    int32_t (*engine_pid)(void *ctx, uint64_t handle);
    void (*release)(void *ctx);
} IkariShellHost;

// shell を作る（エンジンにはまだつながない）。host の所有権は成否にかかわらず Rust に移る（失敗なら release を呼んでから返る）。
// 失敗なら NULL を返し、*err にエラー文（UTF-8、ユーザーの入力は含まない）を入れる。err は NULL でもよい。
IkariShell *ikari_shell_new(const IkariShellConfig *config, IkariShellHost host, IkariShellBuf *err);

// event（ShellEvent の protobuf）を 1 つ処理し、*actions に ShellActions の protobuf を入れる。
// IKARI_SHELL_OK 以外でも（actions が NULL のときを除き）*actions には必ず有効な ShellActions が入る
// （誤りの回は pass_through の 1 つ。キーをアプリに渡して入力を止めない）。
int32_t ikari_shell_dispatch(IkariShell *s, IkariStr event, IkariShellBuf *actions);

// エンジンへの接続 generation（engine_connect が返した番号）が落ちた。**どのスレッドからでも呼べる**。
// 古い接続の通知、解放ずみの shell、範囲外の kind は無視する。
void ikari_shell_engine_down(IkariShell *s, uint64_t generation, uint8_t kind);

// NULL、解放ずみは何もしない（二重の free は安全）。コールバックの中から呼んでもよい（dispatch が返ってから片付ける）。
void ikari_shell_free(IkariShell *s);

// bytes を複製した、Rust が確保したバイト列を返す。ホストのコールバックが *out / *reply に入れるのに使う。
IkariShellBuf ikari_shell_buf_copy(IkariStr bytes);

// ---------------------------------------------------------------- 設定アプリ（別のプロセス）用。§2.3
// IKARI_SHELL_OK 以外では *out は空（ptr が NULL）。

// settings（settings.tsv の文字列）を読み、*out に SettingsValidation の protobuf を入れる。
int32_t ikari_settings_validate(IkariStr settings, IkariShellBuf *out);

// file（今の settings.tsv の文字列）に changes（SettingsChanges の protobuf）を当て、*out に SettingsRewrite の
// protobuf を入れる（成功なら text、警告が増えるなら issues_added。どちらも IKARI_SHELL_OK）。
int32_t ikari_settings_rewrite(IkariStr file, IkariStr changes, IkariShellBuf *out);

// ---------------------------------------------------------------- 候補ウィンドウの配置（L-E で追加）
// 状態を持たない純関数。どのスレッドからでも呼べる。入出力は protobuf。IKARI_SHELL_OK 以外では *out は空。

// request（PanelAnchorRequest）から使うアンカーを選び、*out に PanelAnchor を入れる。
int32_t ikari_panel_resolve_anchor(IkariStr request, IkariShellBuf *out);

// request（PanelLayoutRequest）からパネルの位置と上下を決め、*out に PanelPlacement を入れる。
int32_t ikari_panel_layout(IkariStr request, IkariShellBuf *out);

#ifdef __cplusplus
}
#endif

#endif  // IKARI_SHELL_H
