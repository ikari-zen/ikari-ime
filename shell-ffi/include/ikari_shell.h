// ikari-shell-ffi の C ABI（ADR 0004 §4.3。契約）。殻（IME のプロセス）がローマ字・かな入力と字種変換に使う。
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
void ikari_shell_buf_free(IkariShellBuf buf);

#ifdef __cplusplus
}
#endif

#endif  // IKARI_SHELL_H
