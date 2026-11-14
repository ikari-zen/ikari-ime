/* ikari_candidate.dll の C の ABI（ADR 0062 §7.3。契約。src/abi.rs と同じに保つ）。
 * 受け渡しは数と、proto のバイト列（ikari.candidate.v1.RenderState）だけ。 */
#ifndef IKARI_CANDIDATE_H
#define IKARI_CANDIDATE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IKARI_CANDIDATE_ABI_VERSION 1
#define IKARI_CANDIDATE_OK 0
#define IKARI_CANDIDATE_NOT_IMPLEMENTED (-1)

typedef struct IkariCandidateCore IkariCandidateCore;

/* 描く状態が変わった知らせ。state は RenderState のバイト列で、この呼び出しの間だけ有効。核のスレッドから呼ばれる。 */
typedef void (*IkariCandidateRenderStateCallback)(const uint8_t *state, size_t len, void *user_data);

uint32_t ikari_candidate_abi_version(void);

/* 核を起動する。失敗は NULL。戻り値は ikari_candidate_stop で解放する。 */
IkariCandidateCore *ikari_candidate_start(IkariCandidateRenderStateCallback callback, void *user_data);
int32_t ikari_candidate_stop(IkariCandidateCore *core);

/* パネル全体の高さ（DIP）。行数と行の種類だけで決まる。 */
double ikari_candidate_panel_height_dip(uint32_t candidate_rows, bool has_annotation, bool has_developer_note);

/* 窓の左端の x（物理のピクセル）。 */
double ikari_candidate_horizontal_origin(double anchor_x, double panel_width, double work_x, double work_width);

#ifdef __cplusplus
}
#endif

#endif
