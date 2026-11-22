// ikari-shell-ffi の C ABI（ADR 0004 §4.3、ADR 0041 §4.4。契約）。殻（IME のプロセス）と設定アプリが使う。
// - ikari_shell_*、ikari_settings_*、ikari_panel_*: 殻の OS に依存しない処理（shell-core）への口（ADR 0041）
//   （旧い入力処理の口 ikari_composer_*・ikari_convert_script は、移行の片付け M5 で消した）
// 辞書や llama.cpp には依存しない。Rust の panic は FFI の外へ出ない。
#ifndef IKARI_SHELL_H
#define IKARI_SHELL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// 借用。呼び出しの間だけ有効（UTF-8、NUL 終端なし）。
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
    IKARI_SHELL_ERR_ARG = 7,             // 引数が使えない（ikari_settings_fetch_start の kind や文字列。ADR 0079 §3.6）
    IKARI_SHELL_ERR_FAILED = 8,          // 始められない・できない（起こせない失敗など。文は msg に入る）
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
    IkariStr diag_dir;       // 診断ログのディレクトリ（<UserDir>/logs）。起動中の印 shell.run を置く。空なら印もシグナルの処理も持たない（ADR 0051）
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

// 殻が正常に終わる（applicationWillTerminate）ことを印に残す（ADR 0051。exit=clean）。1 回呼べばよい。
// NULL、解放ずみ、diag_dir が空の shell には何もしない。ikari_shell_free の前に呼ぶ。
void ikari_shell_note_exit(IkariShell *s);

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

// 設定アプリの Model の検証・整形・変更の組み立て（L-C 後半で追加）。request（ikari.settings.v1.AppRequest。
// proto/ikari/settings/v1/settings.proto）を処理し、*out に AppReply を入れる。要求の欄が足りない・範囲外なら
// IKARI_SHELL_ERR_DECODE。
int32_t ikari_settings_app(IkariStr request, IkariShellBuf *out);

// ---------------------------------------------------------------- 取得（ikari-data）の起動（ADR 0079 §3.6。契約 #489）
// 設定アプリ（別のプロセス）用。同じ IkariSettingsFetch を複数のスレッドから同時に呼ばない。
// 画面は、ボタンから start、タイマーで poll、取り消しで cancel、終わったら free を呼ぶだけ。文は Rust が作る。
// 文字列の引数は、NULL（長さ 0）か読める UTF-8（呼び出しの間だけ借りる）。使えなければ IKARI_SHELL_ERR_ARG。
// msg と text は NULL なら書かない。書いたものは ikari_shell_buf_free で 1 回だけ解放する。
// 版（ADR 0080 §7.1）: 口は両方の版で同じ。オフライン版（Rust の feature online なし）は何も起こさず、start は
//   IKARI_SHELL_ERR_FAILED と文を返してハンドルを作らない。poll・sha256・cancel は NULL なら IKARI_SHELL_ERR_NULL、
//   ほかは IKARI_SHELL_ERR_FAILED。画面は ikari_settings_edition_capabilities で取得の入口を出すかを決める。

// 走っている取得の不透明なハンドル。ikari_settings_fetch_free で必ず解放する。
typedef struct IkariSettingsFetch IkariSettingsFetch;

// ikari_settings_fetch_start の kind（値は Windows の ikari_settings_core.h と同じ）。
typedef enum {
    IKARI_SETTINGS_FETCH_PRESET = 1,  // 表のモデル（a が ID）
    IKARI_SETTINGS_FETCH_URL = 2,     // 自由指定の URL（a が URL。format が要る。sha256 と name は空でよい）
} IkariSettingsFetchKind;

// ikari_settings_fetch_poll が書く様子（Windows と同じ形）。
typedef struct {
    int32_t phase;      // 0 取得中、1 取得できた、2 失敗、3 取り消した
    uint64_t received;  // 受けたバイト
    uint64_t total;     // 全体のバイト（has_total が 0 なら使わない）
    uint8_t has_total;
} IkariSettingsFetchStatus;

// 取得を始める。IKARI_SHELL_OK なら *out に値を書く。失敗（IKARI_SHELL_ERR_FAILED）は *msg に文を書く。
// out が NULL なら IKARI_SHELL_ERR_NULL、引数が使えなければ IKARI_SHELL_ERR_ARG。
int32_t ikari_settings_fetch_start(int32_t kind,
    const uint8_t *a, size_t a_len, const uint8_t *format, size_t format_len,
    const uint8_t *sha256, size_t sha256_len, const uint8_t *name, size_t name_len,
    IkariSettingsFetch **out, IkariShellBuf *msg);

// 版の確認（ikari-data version-check。ADR 0076 §6）を始める。様子と文は poll、解放は free（取得と同じ）。
int32_t ikari_settings_version_check_start(IkariSettingsFetch **out, IkariShellBuf *msg);

// 様子を読む。status に数、text に文。fetch が NULL なら IKARI_SHELL_ERR_NULL。
int32_t ikari_settings_fetch_poll(const IkariSettingsFetch *fetch,
    IkariSettingsFetchStatus *status, IkariShellBuf *text);

// 取得が終わったときの sha256（64 桁の 16 進）を *out に書く。まだ終わっていなければ IKARI_SHELL_ERR_FAILED。
int32_t ikari_settings_fetch_sha256(const IkariSettingsFetch *fetch, IkariShellBuf *out);

// 取り消す（子に SIGKILL）。終わっていれば何も変えない。
int32_t ikari_settings_fetch_cancel(const IkariSettingsFetch *fetch);

// 解放する（走っていれば取り消す）。NULL は何もしない。同じ値を 2 回渡さない。
void ikari_settings_fetch_free(IkariSettingsFetch *fetch);

// ---------------------------------------------------------------- 版と、画面に出してよい機能（ADR 0080 §7.2）
// 判断は Rust（shell-core の edition）。画面は値でボタンを出す・出さないを決めるだけ。形と値は Windows と同じ。

typedef enum {
    IKARI_EDITION_NORMAL = 0,   // 通常版
    IKARI_EDITION_OFFLINE = 1,  // オフライン版（通信する機能を持たない）
} IkariEdition;

// 真偽は 0 か 1。
typedef struct {
    int32_t edition;          // IkariEdition
    uint8_t url_model_fetch;  // 「URL から取る」
    uint8_t version_check;    // 「新しい版を確認」
    uint8_t file_import;      // 「ファイルから取り込む」（両方の版で 1）
} IkariEditionCapabilities;

// *out に書く。out が NULL なら IKARI_SHELL_ERR_NULL。どのスレッドからでも呼べる。
int32_t ikari_settings_edition_capabilities(IkariEditionCapabilities *out);

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
