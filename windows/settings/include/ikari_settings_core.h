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
/* 書き出しの既定の名前（ikari-export-YYYYMMDD.tsv）。 */
int32_t ikari_settings_core_export_file_name(uint32_t year, uint32_t month, uint32_t day, IkariSettingsBuf *out);

#ifdef __cplusplus
}
#endif

#endif
