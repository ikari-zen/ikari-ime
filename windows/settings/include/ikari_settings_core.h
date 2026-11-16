/* ikari_settings_core.dll の C の ABI（ADR 0071 §2。契約。src/abi.rs と同じに保つ）。
 * 受け渡しは数と、核が確保したバイト列（ikari_settings_core_buf_free で返す）だけ。 */
#ifndef IKARI_SETTINGS_CORE_H
#define IKARI_SETTINGS_CORE_H

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

#ifdef __cplusplus
}
#endif

#endif
