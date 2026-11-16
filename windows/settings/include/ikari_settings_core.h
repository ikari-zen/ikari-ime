/* ikari_settings_core.dll の C の ABI（ADR 0071 §2。契約。src/abi.rs と同じに保つ）。
 * 受け渡しは数と、核が確保したバイト列（ikari_settings_core_buf_free で返す）だけ。 */
#ifndef IKARI_SETTINGS_CORE_H
#define IKARI_SETTINGS_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IKARI_SETTINGS_CORE_ABI_VERSION 1
#define IKARI_SETTINGS_CORE_OK 0
#define IKARI_SETTINGS_CORE_ERR_NULL (-1)
#define IKARI_SETTINGS_CORE_ERR_PANIC (-2)
#define IKARI_SETTINGS_CORE_ERR_ELEVATED (-3)

typedef struct IkariSettingsBuf {
    uint8_t *ptr;
    size_t len;
} IkariSettingsBuf;

uint32_t ikari_settings_core_abi_version(void);

/* 画面を続けてよいか。高い権限（または確かめられない）なら IKARI_SETTINGS_CORE_ERR_ELEVATED。 */
int32_t ikari_settings_core_check_self(void);

/* 画面に出す設定の一覧（UTF-8 の TSV。列: キー、種類、節、画面、既定値、反映）。out は ikari_settings_core_buf_free で返す。 */
int32_t ikari_settings_core_specs(IkariSettingsBuf *out);

/* 核が返したバイト列を返す。ptr が NULL なら何もしない。 */
void ikari_settings_core_buf_free(IkariSettingsBuf buf);

/* ---- 設定の口へのつながり（src/session_abi.rs。W4-s3 の第 2 段）----
 * 文字列は UTF-8 のポインタと長さ（呼び出しの間だけ借りる）。msg は結果の欄の文（NULL なら書かない）。 */
#define IKARI_SETTINGS_CORE_ERR_FAILED (-4)
#define IKARI_SETTINGS_CORE_ERR_DISCONNECTED (-5)
#define IKARI_SETTINGS_CORE_ERR_ARG (-6)
#define IKARI_SETTINGS_CORE_WIDGET_BOOL 1
#define IKARI_SETTINGS_CORE_WIDGET_INT 2
#define IKARI_SETTINGS_CORE_WIDGET_CHOICE 3

typedef struct IkariSettingsSession IkariSettingsSession;

/* つなぐ（相手が engine-host かを照らしてから Hello）。失敗は NULL で、out_status に状態の番号。close で解放する。 */
IkariSettingsSession *ikari_settings_core_open(int32_t *out_status);
void ikari_settings_core_close(IkariSettingsSession *session);
/* 状態の番号の文（状態の欄に出す）。 */
int32_t ikari_settings_core_status_text(int32_t status, IkariSettingsBuf *out);
/* 画面の行（TSV。列: キー、部品、節、画面、値、下限、上限、選択肢、反映）。session が NULL なら既定値。 */
int32_t ikari_settings_core_view(const IkariSettingsSession *session, IkariSettingsBuf *out, IkariSettingsBuf *msg);
/* 書き換え。widget は IKARI_SETTINGS_CORE_WIDGET_*。真偽は number の 0/1、数は number、選択肢は choice。 */
int32_t ikari_settings_core_edit(const IkariSettingsSession *session, const uint8_t *key, size_t key_len, int32_t widget,
                                 int64_t number, const uint8_t *choice, size_t choice_len, IkariSettingsBuf *msg);
int32_t ikari_settings_core_reset(const IkariSettingsSession *session, const uint8_t *key, size_t key_len,
                                  IkariSettingsBuf *msg);
/* パスでの書き出し・取り込み（ADR 0068 §4）。replace は上書きの確認を済ませたとき。dry_run は確かめるだけ。 */
int32_t ikari_settings_core_export(const IkariSettingsSession *session, const uint8_t *path, size_t path_len, bool replace,
                                   IkariSettingsBuf *msg);
int32_t ikari_settings_core_import(const IkariSettingsSession *session, const uint8_t *path, size_t path_len, bool dry_run,
                                   IkariSettingsBuf *msg);
/* 自由指定のモデルをファイルから取り込む（ADR 0076 §7。パスだけを送る）。sha256 と name は空でよい。（契約。D-4） */
int32_t ikari_settings_core_import_model(const IkariSettingsSession *session, const uint8_t *path, size_t path_len,
                                         const uint8_t *format, size_t format_len, const uint8_t *sha256,
                                         size_t sha256_len, const uint8_t *name, size_t name_len, IkariSettingsBuf *msg);
/* 自由指定のモデルのボタンを出してよいか（1 / 0）。判断は核（段 2 のある OS だけ）。session は NULL でよい。（契約。D-4） */
int32_t ikari_settings_core_custom_model_offered(const IkariSettingsSession *session);
/* 書き出しの既定の名前（ikari-export-YYYYMMDD.tsv）。 */
int32_t ikari_settings_core_export_file_name(uint32_t year, uint32_t month, uint32_t day, IkariSettingsBuf *out);

/* ---- 取得（ikari-data.exe）の起動、進み具合、取り消し（src/fetch_abi.rs。ADR 0076）----
 * 画面はボタンで start、タイマーで poll、取り消しで cancel、終わったら free。文は核が作る。 */
#define IKARI_SETTINGS_FETCH_PRESET 1 /* a = 表のモデルの ID */
#define IKARI_SETTINGS_FETCH_URL 2    /* a = https の URL。format が要る。sha256 と name は空でよい */
#define IKARI_SETTINGS_FETCH_PHASE_RUNNING 0
#define IKARI_SETTINGS_FETCH_PHASE_DONE 1
#define IKARI_SETTINGS_FETCH_PHASE_FAILED 2
#define IKARI_SETTINGS_FETCH_PHASE_CANCELLED 3

typedef struct IkariSettingsFetch IkariSettingsFetch;

typedef struct IkariSettingsFetchStatus {
    int32_t phase;
    uint64_t received;
    uint64_t total; /* has_total が 0 なら使わない */
    uint8_t has_total;
} IkariSettingsFetchStatus;

/* 始める。成功で *out に値（free で解放）。ERR_ARG は引数、ERR_FAILED は起こせない（msg に文）。 */
int32_t ikari_settings_fetch_start(int32_t kind, const uint8_t *a, size_t a_len, const uint8_t *format, size_t format_len,
                                   const uint8_t *sha256, size_t sha256_len, const uint8_t *name, size_t name_len,
                                   IkariSettingsFetch **out, IkariSettingsBuf *msg);
/* 様子。status に数、text に文（NULL なら書かない）。 */
int32_t ikari_settings_fetch_poll(const IkariSettingsFetch *fetch, IkariSettingsFetchStatus *status, IkariSettingsBuf *text);
int32_t ikari_settings_fetch_cancel(const IkariSettingsFetch *fetch);
void ikari_settings_fetch_free(IkariSettingsFetch *fetch);
/* 終わった取得の sha256（64 桁の 16 進）。終わっていなければ ERR_FAILED。sha256 なしで取ったときの確認に出す。（契約。D-4） */
int32_t ikari_settings_fetch_sha256(const IkariSettingsFetch *fetch, IkariSettingsBuf *out);
/* 版の確認を始める（ikari-data version-check。ADR 0076 §6）。様子は fetch_poll、解放は fetch_free。（契約。D-2d） */
int32_t ikari_settings_version_check_start(IkariSettingsFetch **out, IkariSettingsBuf *msg);

#ifdef __cplusplus
}
#endif

#endif
