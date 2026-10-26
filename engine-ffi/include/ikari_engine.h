// ikari-engine-ffi の C ABI（ADR 0004 §4.2。契約）。
// メッセージは protobuf のバイト列（proto/ikari/engine/v1/engine.proto）。
// この呼び出しの外へ Rust の panic は出ない（INTERNAL の ErrorReply になる）。
#ifndef IKARI_ENGINE_H
#define IKARI_ENGINE_H

#include <stddef.h>
#include <stdint.h>

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

#ifdef __cplusplus
}
#endif

#endif  // IKARI_ENGINE_H
