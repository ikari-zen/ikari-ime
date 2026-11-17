// ikari-engine-ffi の C ABI（ADR 0004 §4.2。契約）。
// メッセージは protobuf のバイト列（proto/ikari/engine/v1/engine.proto）。
// この呼び出しの外へ Rust の panic は出ない（INTERNAL の ErrorReply になる）。
#ifndef IKARI_ENGINE_H
#define IKARI_ENGINE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __APPLE__
#include <xpc/xpc.h>
#endif

#ifdef __cplusplus
extern "C" {
#endif

// 不透明なエンジンのハンドル
typedef struct IkariEngine IkariEngine;

// Rust が確保したバイト列。ikari_buf_free で1回だけ解放する（len == 0 でも解放する）。
typedef struct {
    uint8_t *ptr;
    size_t len;
} IkariBuf;

// config: EngineConfig（protobuf）。失敗なら NULL を返し、*err に ErrorReply（protobuf）を入れる。
// err は NULL でもよい（入れない）。成功のとき *err は触らない。
IkariEngine *ikari_engine_open(const uint8_t *config, size_t len, IkariBuf *err);

// req: Envelope（protobuf）。返り値も Envelope。決して NULL にせず、エラーも Envelope で返す。
// 複数スレッドから同時に呼んでよい。Convert は締め切りまでこのスレッドを止める。
IkariBuf ikari_engine_handle(IkariEngine *e, const uint8_t *req, size_t len);

void ikari_buf_free(IkariBuf buf);

// 学習を保存してから解放する。NULL は何もしない。close のあと e を使わない。
void ikari_engine_close(IkariEngine *e);

// proto の schema_version（Envelope.schema_version と同じ値）
uint32_t ikari_engine_schema_version(void);

#ifdef __APPLE__
// 殻が仲立ちした檻役の匿名の口を渡す（macOS。ADR 0077 §3.5）。requirement は口の先（檻役）の署名要件。
// エンジンが口を retain して持つ（呼び出し側の参照はそのまま）。口の確かめ（hello）が終わるまで戻らない。
// 返り値は「受け取って確かめた」か（確かめに通ったかではない。通らなければ、エンジンは口を捨てて今の口のままにする）。
// false は、引数が使えない（NULL、UTF-8 でない、口でない）とき。
bool ikari_engine_attach_lm_cage(IkariEngine *e, xpc_endpoint_t endpoint, const char *requirement);
#endif

#ifdef __cplusplus
}
#endif

#endif  // IKARI_ENGINE_H
