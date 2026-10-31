# 入力中の 1 打（ローマ字）の費用（2026-10-31。issue #161、直す順番の 2 の測定）

main 0da0651 の上に、入力中の台本を測り手に足しただけのコミット。記録: `alloc-shell-typing-2026-10-31.md`（確保の表。ほかに定常・最初の 1 打・起動の表）、`cpu-shell-typing-2026-10-31.md`。負荷平均は確保 3.5（確保の回数は負荷に左右されない）、CPU 2.6、P1 約 2.6〜3.0。測定中に別のプロセス（VS Code の拡張）が CPU を使っていた。時間は参考。

## 測り方
- 台本（`shell-core/benches/shell_perf.rs` の `TYPING_SCRIPTS`、測り手専用。共通シナリオは増やしていない）: ローマ字で 6 本。母音、子音+母音、促音（kk、tt）、撥音（nn）、拗音（kya、jyu、tyo）、長い読み（約 30 打）、Backspace 3 回、Esc。1 台本ごとに新しい `Shell` を作り、`Activate` のあと打つ。
- 1 打を、最初に一度流して決める 3 区分に分ける: **ローマ字の途中**（読みが変わらず、先読みを送らない。例: `k`、`ky`）、**読みが変わる**（かなが増えて先読みを送る）、**Backspace、Esc**。区分ごとに別の関数で呼ぶので、`perf alloc`（既定で表が増える）と `perf profile --focus shell_perf::typing_` で分けて見られる。
- 既存の指標（`p50_ns` など）は変えていない。JSON に `typing`（`pending`、`reading`、`edit`）を足した。

## 入力中の 1 打の実際の費用
| 区分 | dispatch | 確保 p50 / p99 / max | P1 p50 | P1 p99 |
|---|---:|---|---:|---:|
| ローマ字の途中 | 188000 | 9 / 21 / 33 | 458〜500 ns | 0.9〜1.2 us |
| 読みが変わる | 244000 | 11 / 26 / 38 | 790〜920 ns | 5.0〜5.3 us |
| Backspace、Esc | 16000 | 8 / 72 / 72 | 625〜667 ns | 5.5〜6.1 us |

変換中の定常（確保 p50 13、p99 126）より、入力中の 1 打は、確保が p50 で同程度、p99 で 1/5 と小さい。時間は 0.5〜0.9 us で、変換中（p50 750 ns）と同じ桁。

## 確保の場所（入力中。1 回の dispatch あたり）
| 場所 | ローマ字の途中 | 読みが変わる |
|---|---:|---:|
| `KeyKind::clone`（キー変換。#161 の順番 3） | 2.00 | 2.00 |
| `RomajiComposer::push`（input/src/romaji.rs:131、135、142） | 1.89 | 2.33 |
| `refresh_composing`（mod.rs:755 `kana().to_string()`、756 `pending().to_string()`、772 `format!`） | 2.85 | 2.10 |
| `prefetch_if_changed`（mod.rs:2066、2069 `reading.to_string()` × 2） | 0（等しいので早く戻る） | 2.00 |
| `Envelope::encode_to_vec`（先読みの符号化。本番でも要る） | 0 | 1.00 |
| `menu::*`、`accept` | 0.4〜0.5（各台本の最初の 1 打に出る起動の費用。定常ではない） | 0.2 |

`refresh_composing` の `format!`（mod.rs:772）は 1 回 80 バイトで、バイト数が最大（15〜20 MB/測定）。

## CPU（入力中。samply、サンプル 5067）
- 呼び出しを含む時間: `refresh_composing` **60.2%**。うち `EngineLink::post`／`prefetch` 31〜32%。ただしこの中に**測り手の替え玉のエンジン**（`ScriptedTransport::post`。要求の復号と台本の照合）が 22.6% 入る。本番ではエンジンは別のプロセスなので、殻自身の先読みの費用は約 10% と見る（推測。引き算）。
- `RomajiComposer::push`（`feed` 経由）19.0%。うち BTreeMap の規則の検索（`ikari_input::romaji::Rule` の `NodeRef` 検索）が 8.9% + 7.8%。
- 自分の時間: `malloc` 22〜24%、`Graphemes::next` **9.8〜11.1%**（`grapheme_count`。`prefetch_if_changed` が**ローマ字の途中の打鍵でも毎回**、読み全体を走査してから「前回と同じ」を判定する。呼び出し元の確認は、コードを読んだ上の推測）。
- **`is_valid_reading`（codec::chars）は上位に出ない**（0.2%。`do_count_chars`）。前の測定の 10.8%、23.2% は、変換中のシナリオの長い読みによるものだった。

## PR D の対象（定常の入力中として本当に上位か）
| 対象 | 上位か | 見立て |
|---|---|---|
| `grapheme_count`（`prefetch_if_changed`） | **上位**（CPU の自分の時間 約 10%） | 読みが同じ打鍵では比べる前に数えている。「前回と同じ」を先に判定する、ASCII だけなら `len()` で数える、で消せる見込み（結果は同じ） |
| `refresh_composing` の `to_string` ×2 と `format!` | **上位**（確保 約 2〜3 回/打、バイト数最大） | `String` を使い回す（`clone_from`）、`kana` が変わらなければ再利用 |
| `prefetch_if_changed` の `reading.to_string()` ×2 | 中（読みが変わる打鍵で 2 回） | 1 回にできる（`last_prefetched_reading` と要求で共有） |
| `is_valid_reading` | **外す**（0.2%） | 直さない |
| `RomajiComposer::push`（input crate） | 上位（CPU 19%、確保 約 2 回） | D の範囲外（`ikari_input` のロジック。別の PR で判断を仰ぐ） |
| `KeyKind::clone` | 確保 2.0 回/打（1 バイト） | #161 の順番 3（別の PR） |

## 推測と注意
- 台本は 6 本、約 100 打で、日本語の実際の打鍵の分布ではない。区分の比率（途中 44%、読みが変わる 52%、編集 4%）は台本の都合。
- 呼び出し元の分解（grapheme_count が `prefetch_if_changed` から来る、など）は、dhat の呼び出しの列と samply の記号表では確かめていない。symbolication が一部（`String::as_str` が 30% など）ずれて見える。D で直して、前後の差で確かめる。
