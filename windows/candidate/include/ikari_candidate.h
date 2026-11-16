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
#define IKARI_CANDIDATE_ERROR (-2)
/* ikari_candidate_vertical_origin の side（proto の PanelSide と同じ番号） */
#define IKARI_CANDIDATE_SIDE_BELOW 1
#define IKARI_CANDIDATE_SIDE_ABOVE 2

typedef struct IkariCandidateCore IkariCandidateCore;

/* 描く状態が変わった知らせ。state は RenderState のバイト列で、この呼び出しの間だけ有効。核のスレッドから呼ばれる。 */
typedef void (*IkariCandidateRenderStateCallback)(const uint8_t *state, size_t len, void *user_data);

uint32_t ikari_candidate_abi_version(void);

/* 核を起動する（このユーザーとセッションの候補窓のパイプで受け付ける）。失敗、高い権限、Windows 以外は NULL。
 * 戻り値は ikari_candidate_stop で解放する。知らせの中で核の関数を呼ばない。 */
IkariCandidateCore *ikari_candidate_start(IkariCandidateRenderStateCallback callback, void *user_data);
int32_t ikari_candidate_stop(IkariCandidateCore *core);

/* パネル全体の高さ（DIP）。行数と行の種類だけで決まる。 */
double ikari_candidate_panel_height_dip(uint32_t candidate_rows, bool has_annotation, bool has_developer_note);

/* 窓の左端の x（物理のピクセル）。 */
double ikari_candidate_horizontal_origin(double anchor_x, double panel_width, double work_x, double work_width);

/* 窓の上端の y（物理のピクセル。y は下向き）。side は TIP が決めたもの（IKARI_CANDIDATE_SIDE_*。知らない値は下）。
 * 作業領域の外に出さない。 */
double ikari_candidate_vertical_origin(double anchor_y, double anchor_height, double panel_height, int32_t side,
                                       double work_y, double work_height);

#ifdef __cplusplus
}
#endif

#endif
