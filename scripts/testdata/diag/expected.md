## 直し方の分布

| 項目 | 件数 | 率 |
|---|---|---|
| sh_episode 全体 | 7 |  |
| 確定した合成（再変換を除く） | 3 |  |
| 1 位のまま確定 | 1 | 33.3% |
| 1 位から外れた | 2 | 66.7% |
| 再変換から始まった確定 | 1 |  |
| pick=0 | 1 | 50.0% |
| pick=1 | 1 | 50.0% |
| pick=2 | 0 | 0.0% |
| pick=3 | 0 | 0.0% |
| pick=4 | 0 | 0.0% |
| pick=5 | 0 | 0.0% |
| pick=6 | 0 | 0.0% |
| pick=7 | 0 | 0.0% |
| pick=8 | 0 | 0.0% |
| pick=9+ | 0 | 0.0% |
| spaces=0 | 0 | 0.0% |
| spaces=1 | 1 | 33.3% |
| spaces=2 | 1 | 33.3% |
| spaces=3 | 1 | 33.3% |
| spaces=4+ | 0 | 0.0% |
| ページを進めた合成 | 1 | 33.3% |
| 数字キーで選んだ合成 | 0 | 0.0% |
| 文節のモード（フォーカス移動あり）の合成 | 1 | 33.3% |
| seg_focus の合計回数 | 2 |  |
| 区切り直しあり合成 | 1 | 33.3% |
| seg_resize の合計回数 | 1 |  |
| Esc で読みに戻した合成 | 0 | 0.0% |
| esc_back の合計回数 | 0 |  |
| 2 文節以上で確定 | 1 | 33.3% |

## 1 位のまま確定した率（日ごと）

| 日(UTC) | 確定 | 1位のまま | 率 |
|---|---|---|---|
| 2026-10-26 | 2 | 1 | 50.0% |
| 2026-10-27 | 1 | 0 | 0.0% |

## 終わった理由の内訳

| end | mode | 件数 | 率 |
|---|---|---|---|
| commit_cand | romaji | 3 | 42.9% |
| commit_composition | shortcode_en | 1 | 14.3% |
| commit_segments | romaji | 1 | 14.3% |
| esc_clear | kana | 1 | 14.3% |
| shortcode_pick | shortcode_en | 1 | 14.3% |
| commit_composition の直前 | shortcode | 1 | 100.0% |

## 入力の流れ

| 項目 | 件数 | p50 | p90 |
|---|---|---|---|
| rlen | 3 | 6 | 9 |
| keys_to_conv | 3 | 8 | 11 |
| dur_ms | 6 | 900 | 4000 |
| pick_ms | 3 | 830 | 2500 |
| sh_mode ja->en | 1 |  |  |

## 機能の利用

| 項目 | 件数 |
|---|---|
| pick_kind=dict | 2 |
| pick_kind=emoji | 1 |
| pick_kind=rewriter | 1 |
| rw=postal | 1 |
| mode=shortcode_* の合成 | 2 |
| うち end=shortcode_pick | 1 (50.0%) |
| mode=kana | 1 |
| mode=romaji | 4 |
| mode=shortcode_en | 2 |

## 不具合の手がかり

| 項目 | 件数 |
|---|---|
| ctx=ok | 3 |
| panel=client | 1 |
| panel=last_known | 2 |
| panel=none | 1 |
| convert/prefetch の ctx=0 | 1 (20.0%) |
| sh_engine timeout 合計 | 2 |
| sh_engine interrupted 合計 | 0 |
| sh_engine invalidated 合計 | 0 |
| sh_engine fallback_convs 合計 | 1 |
| sh_engine breaker_open 合計 | 0 |
| sh_engine reconnect 合計 | 0 |
| sh_engine resync 合計 | 0 |
| err=UNKNOWN_SESSION (commit_text) | 1 |
| err=UNKNOWN_SESSION (convert) | 2 |
| err=UNKNOWN_SESSION (prefetch) | 1 |
| err=UNKNOWN_SESSION (reconvert) | 1 |

## UNKNOWN_SESSION の内訳

| 項目 | 件数 | 率 |
|---|---|---|
| UNKNOWN_SESSION の行 | 5 |  |
| kind=commit_text | 1 | 20.0% |
| kind=convert | 2 | 40.0% |
| kind=prefetch | 1 | 20.0% |
| kind=reconvert | 1 | 20.0% |
| lm=loading の行 | 4 | 80.0% |
| エンジンの起動（epoch）の数 | 2 |  |
| 直前の end_session からの時間: 1 秒未満 | 2 | 40.0% |
| 直前の end_session からの時間: 10 秒未満 | 2 | 40.0% |
| 直前の end_session からの時間: なし | 1 | 20.0% |
| 直前の begin_session からの時間: 1 秒未満 | 3 | 60.0% |
| 直前の begin_session からの時間: 10 秒未満 | 1 | 20.0% |
| 直前の begin_session からの時間: なし | 1 | 20.0% |
| エンジンの起動（epoch の先頭）からの時間: 1 秒未満 | 4 | 80.0% |
| エンジンの起動（epoch の先頭）からの時間: 10 分以上 | 1 | 20.0% |
| session= のある行（end_session と同じ版以降のエンジン） | 3 | 60.0% |
| session= の状態: end_session のあと | 1 | 33.3% |
| session= の状態: このエンジンの起動で begin_session なし | 2 | 66.7% |
| かたまり（行の間隔が 5 秒以内）の数 | 1 |  |
| かたまりの直前の行の kind=convert | 1 | 100.0% |
| かたまりの直前の無音: 1 秒未満 | 1 | 100.0% |
| かたまりの先頭がエンジンの起動の最初の行 | 0 | 0.0% |
| かたまりの前で最後の出来事が end_session | 0 | 0.0% |

## アプリ別の convert と miss=context

| app | convert | miss=context | 率 | lm_applied=0 | 率 |
|---|---|---|---|---|---|
| (不明) | 3 | 1 | 33.3% | 1 | 33.3% |
| com.example.Editor | 1 | 0 | 0.0% | 0 | 0.0% |
| com.example.Terminal | 1 | 0 | 0.0% | 0 | 0.0% |

## 異常終了

| 項目 | 件数 |
|---|---|
| エンジンの前回の終わり方の記録 | 4 |
| エンジン exit=clean | 1 |
| エンジン exit=term | 1 |
| エンジン exit=crash | 1 |
| エンジン exit=unknown | 1 |
| エンジン sig=11 | 1 |
| エンジン sig=15 | 1 |
| エンジン 2026-10-28 exit=crash | 1 |
| エンジン 2026-10-28 exit=unknown | 1 |
| エンジン 2026-10-29 exit=clean | 1 |
| エンジン 2026-10-29 exit=term | 1 |
| 殻の前回の終わり方の記録 | 2 |
| 殻 exit=clean | 0 |
| 殻 exit=term | 0 |
| 殻 exit=crash | 1 |
| 殻 exit=unknown | 1 |
| 殻 sig=11 | 1 |
| 殻 2026-10-28 exit=unknown | 1 |
| 殻 2026-10-29 exit=crash | 1 |
| エンジン exit=crash の直前の convert app=com.example.Editor | 1 |
| 殻 exit=crash の直前の convert app=com.example.Terminal | 1 |
| エンジンの exit=unknown のうち、殻の exit=unknown が 2 分以内にあるもの | 1 |
| エンジン panic 合計 | 2 |
| エンジン panic loc=engine/src/lib.rs:120 | 2 |
| 殻 sh_panic 合計 | 3 |
| 殻 sh_panic loc=shell-core/src/engine/mod.rs:40 | 2 |
| 殻 sh_panic loc=external | 1 |

## 性能（us）

| 項目 | 件数 | p50 | p95 | p99 |
|---|---|---|---|---|
| convert（全体） | 5 | 50 | 5200 | 5200 |
| convert cache=hit lm_applied=1 | 4 | 50 | 60 | 60 |
| convert cache=miss lm_applied=0 | 1 | 5200 | 5200 | 5200 |
| segment_page | 0 | - | - | - |
| page | 0 | - | - | - |
| prefetch_done | 1 | 25000 | 25000 | 25000 |
| sh_episode wait_us | 3 | 6200 | 9000 | 9000 |

## cache と LM

| 項目 | 件数 | 率など |
|---|---|---|
| convert 件数 | 5 |  |
| cache=hit | 4 | 80.0% |
| cache=miss | 1 | 20.0% |
| lm_applied=1 | 4 | 80.0% |
| miss=context | 1 | 20.0% |
| LM なし（lm_applied=0） | 1 | 20.0% |
| by=content | 0 | 0.0% |
| fill のある convert | 0 | 0.0% |
| fill_eq=1 | 0 | - |
| fill_eq=0 | 0 | - |
| fill_eq=0 fill_tail=1 | 0 | - |
| fill_eq=0 fill_tail=0 | 0 | - |
| stat rss_kb 中央値 | 300000 |  |
| stat rss_kb 最大 | 300000 |  |
| lm_load 回数 | 1 | p50=800000us |
| lm_unload reason=shutdown | 1 |  |

## ログの健全性

| 項目 | 値 |
|---|---|
| 読んだファイル | 3 |
| 読めた行 | 46 |
| 形の違う行（欠け） | 1 |
| log_dropped 合計 | 5 |
| diag_reject 合計 | 2 |
| エンジンの起動（epoch） | 4 |
| セッション（epoch, session= の組） | 4 |
| smoke のセッション | 1 |
| 除いた smoke の行 | 4 |
| 期間(UTC) | 2026-10-26 〜 2026-10-29 |
| 行数 2026-10-26 | 7 |
| 行数 2026-10-27 | 23 |
| 行数 2026-10-28 | 8 |
| 行数 2026-10-29 | 4 |
