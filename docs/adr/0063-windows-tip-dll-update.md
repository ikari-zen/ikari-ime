# ADR 0063: Windows の TIP の DLL（`ikari_tip.dll`）の更新。版ごとのフォルダに新しい DLL を置き、COM の登録だけを新しい場所に向け直す。古い版は次の更新か再起動で消す

- 状態: 提案
- 日付: 2026-11-14
- 対象: W5 のインストーラ（未作成。`windows/installer/`）、`windows/tip/src/com/register.rs`（登録）と `com/host.rs`（資源の場所）、`windows/dev/register.ps1`・`unregister.ps1`（開発用の導入）、`docs/windows-real-device-check.md` の項目 29、`proto/ikari/engine/v1/engine.proto`（`schema_version` の運用）
- 前提: main = 0ab78cd。#377（項目 29 の実機の結果）、#331 のコメント（2026-11-14 の VM の確認）、ADR 0040 §2.7・§6 の S9・§7 の W5、ADR 0058（データの束）、ADR 0062（候補窓は別プロセス）、#99（未署名の 0.x の配布）
- 判断の置き場所: Rust とインストーラ。どの版を登録するかは、新しい DLL の `DllRegisterServer`（Rust。自分のパスを書く）。古い版の掃除と再起動の案内はインストーラ（OS の配布の道具。判断は「消せるものを消す」だけ）。TIP とエンジンの版の食い違いは、今の `schema_version` と `shell-core` の「エンジンなし」の動き（Rust）で扱う。Swift は関わらない
- この ADR ではコードを変えない。「分かったこと」はコード、公式の文書、他の IME の公開のソース、実機の結果で確かめたもの。「推測」は確かめていないもの。推測は §7 のスパイクで確かめてから本決定にする

---

## 1. 現状（分かったこと）

### 1.1 実機（2026-11-14、VM の Windows 11 Pro 26300。#331、#377、項目 29）
- 読み込まれている `ikari_tip.dll`（`%LOCALAPPDATA%\ikari-ime-dev\tip\`）への上書き（`Copy-Item -Force`）は失敗する（IOException「別のプロセスで使用されている」）。
- Notepad・Edge を閉じても失敗する。持ち主は `explorer.exe` と `WindowsTerminal.exe`（`tasklist /m ikari_tip.dll`）。
- explorer を再起動しても、新しい explorer が**同じパスの DLL を読み直す**ので、上書きできない。
- 開発の過程の記録では、改名してから新しいものを置き、`regsvr32` する方法は通った（いつ、どの版で、までは残っていない）。
- 同じ日の項目 30: エンジンが mmap している辞書は、上書きも、フォルダの改名もできない（ADR 0058 §14 の推測どおり）。

### 1.2 コード
- 登録（`windows/tip/src/com/register.rs`）: `HKCR\CLSID\{CLSID}\InprocServer32` の既定値に **DLL 自身のパス**を書き、TSF の入力プロファイルとカテゴリを登録する。いまは HKCR（`HKLM\Software\Classes`。管理者が要る）。HKCU で TSF が読むかは未確認（`TODO(S9)`）。
- 資源（キー割り当ての表など）は **DLL と同じ場所の `resources`**（`com/host.rs`。`TODO(W5)`）。DLL と資源は同じフォルダで一緒に動く。
- 開発用の導入（`windows/dev/register.ps1`）: 決まったパスへコピーして `regsvr32 /s`。上書きに失敗したら「アプリを閉じる、サインアウト、再起動」を案内して止まる。
- エンジンとの版: 封筒（envelope）に `schema_version = 1`。違えば `SCHEMA_MISMATCH` を返す。ただし **`Shutdown` だけは版が違っても受ける**（`engine/src/dispatch.rs`。殻が古いエンジンを止めるため）。今までの追加はすべて「追加だけなので 1 のまま」（`engine.proto` の注記）。`Hello` は `shell_version`、返事は `engine_version` の文字列を運ぶが、比べてはいない。
- `shell-core` は `Hello` が失敗したとき（`SCHEMA_MISMATCH` を含む）、LM などの情報なしで続ける（`controller/background.rs`）。エンジンにつながらない間は、エンジンなしの候補（かな）で動く（#329）。
- パイプの相手の確認: `pipe` は相手の PID を返すだけで、「インストール先のパスと一致するか」は TIP が決める（`pipe/src/os/windows.rs` の冒頭）。**TIP の側の照合はまだ無い**（ADR 0040 §2.7 の方針だけがある）。

### 1.3 他の IME（公開のソースで確かめた。2026-11-14）
| IME | 道具 | TIP の DLL の置き方 | 使用中のときの扱い |
| --- | --- | --- | --- |
| weasel（小狼毫） | NSIS（`output/install.nsi`） | **版ごとのフォルダ** `…\Rime\weasel-<版>\`（`WEASEL_ROOT`）に置き、`WeaselSetup.exe` で登録し直す | 更新のとき、古い版のフォルダのファイルを `Delete` で消そうとし（使用中なら残る）、`SetRebootFlag true`。注記「更新の間は TSF のプロファイルを残す。消すと Windows が既定の IME を足す」 |
| Mozc | WiX の MSI（`src/win32/installer/installer_oss_64bit.wxs`） | 決まった名前 `Program Files\Mozc\mozc_tip64.dll` | Windows Installer に任せる（使用中のファイルは再起動のときに置き換える仕組み。下の公式の文書）。`ScheduleReboot` は**削除のときだけ**。登録は独自の custom action（`RegisterTIP64`、失敗したら古い版を登録し直す `RegisterTIPRollback64`）。サーバーは `InstallValidate` の前に止める |
| azooKey-Windows | Inno Setup（`installer/Installer.iss`） | 決まった名前 `{userappdata}\…\azookey.dll`（管理者で入れる） | `ignoreversion regserver` だけ。`restartreplace` は使っていない。`UninstallNeedRestart` は常に真（削除のあとは再起動を求める） |
| Microsoft の TSF のサンプル（Windows-classic-samples の SampleIME） | なし | `regsvr32` で手で登録 | 扱いなし |

- 公式の文書（Microsoft Learn）: `MoveFileExW` の `MOVEFILE_DELAY_UNTIL_REBOOT` は、次の**再起動**で移動・削除する（`PendingFileRenameOperations`。管理者か LocalSystem が要る）。サインアウトでは動かない。Windows Installer は、使用中のファイルを再起動のときに置き換え、再起動を求める。
- azooKey-Windows と Mozc が使用中の DLL に当たったとき、実際にどの画面が出るか（ファイルが使用中の警告か、再起動の要求か）は、動かして確かめていない（推測しない）。

### 1.4 推測（未確認）
- 読み込まれている DLL は、**改名はできる**（ローダーが削除の共有を付けて開く）。§1.1 の開発の記録と合う。
- 登録を別の場所に向け直したあと、すでに読み込んだプロセスは古い版のまま動き、新しく始まるプロセス（explorer の再起動も含む）は新しい版を読む。
- `InprocServer32` のパスは、TSF がプロセスごとに、TIP を作るときに読む。`ctfmon` などが長く覚えておくことはない。
- サインアウトとサインインで、全部のプロセスが新しい版になる。

## 2. 問題
- 配った版を更新するたびに、使用中の DLL に当たる。Windows ではほぼ必ず explorer が持っているので、「閉じてもらえば上書きできる」が成り立たない（§1.1）。
- 決めるのは、(a) 新しい DLL をどこに置き、いつ切り替わるか、(b) 古い版をいつ消すか、(c) 切り替わるまでの間、古い TIP と新しいエンジンが話す場合をどう扱うか、(d) 削除（アンインストール）のとき、(e) 未署名・0.x の配布（#99）の中でどうするか。
- 目標: 更新のあと、**再起動なしで、新しく開いたアプリから新しい版が動く**。古い版が残っていても壊れない（#377 の完了の条件）。入力を止めない（方針の 1 本目）。
- 目標にしない: 自動の更新の確認とダウンロード（ADR 0058 と同じく、0.x では持たない）、インストーラの道具の選択（W5 のインストーラの ADR）、ストアアプリ、ARM64（ADR 0040 §2.3）。

## 3. 決定の要約
1. **TIP の DLL は、版ごとのフォルダに置く**: `<インストール先>\tip\<版>\ikari_tip.dll`（と、その下の `resources\`）。新しい版を新しいフォルダに入れ、**新しい DLL の `DllRegisterServer` を呼んで、`InprocServer32` のパスを新しい場所に書き換える**。古い DLL には触れない（上書きも改名もしない）。weasel と同じ形。
2. **更新のときは登録を解除しない**。CLSID とプロファイルの GUID は版をまたいで変えない。解除してから登録し直すと、利用者の入力方式の一覧から外れる、既定の IME が足される恐れがある（weasel の注記。推測。§7 で確かめる）。
3. **切り替わる時期**: 新しく始まるプロセスから新しい版。すでに動いているプロセス（explorer など）は、そのプロセスが終わるまで古い版。インストーラの最後に「サインアウトしてサインインすると、すべてのアプリで新しい版になります」と出す。再起動は求めない。explorer を勝手に再起動しない。
4. **古い版の掃除はインストーラが行う**: 更新のたびに、今の版より前の `tip\<版>\` を消そうとする。消せないもの（使用中）は `MOVEFILE_DELAY_UNTIL_REBOOT` で次の再起動に消す予約をする。残っても害はない（どこからも登録されていないので、新しいプロセスは読まない）。前の版を残して戻せるようにする仕組みは持たない（戻すなら前のインストーラを入れ直す）。
5. **エンジン（`ikari-engine-host.exe`）と候補窓（`ikari-candidate.exe`）は、決まったパスに置き、上書きする**。これらは自分のプロセスしか持たないので、インストーラが止めて（`Shutdown`。版が違っても受ける）、置き換えて、起動し直せる。パスを版ごとにしないのは、TIP のパイプの相手の確認（インストール先のパス。ADR 0040 §2.7）を、古い TIP からも通すため。
6. **版の食い違い**: 切り替わるまでの間、**古い TIP と新しいエンジン**が話すのは普通に起きる。エンジンは、同じ `schema_version` の間、古い TIP の要求に答え続ける（今の「追加だけなら 1 のまま」の運用を規則にする）。互換を壊す変更は `schema_version` を上げ、古い TIP は `SCHEMA_MISMATCH` で、エンジンなし（かな）で動く（固まらない、落ちない。今の動き）。上げた版のリリースノートには「サインアウトが要る」と書く。古い版を同時に受ける（N と N−1）仕組みは持たない（今は要らない）。
7. **削除（アンインストール）**: `DllUnregisterServer`（CLSID、プロファイル、カテゴリを消す）→ エンジンと候補窓を止める → ファイルを消す。使用中の DLL は `MOVEFILE_DELAY_UNTIL_REBOOT` で予約し、最後に「再起動すると残りのファイルが消えます」と出す（すぐの再起動は求めない）。ユーザーデータ（`%LOCALAPPDATA%\ikari-ime\`。学習、設定、辞書）の扱いは Q3。
8. **開発用の導入（`windows/dev/register.ps1`）も同じ形にする**: `tip\<日時>\` に置いて `regsvr32`。上書きの失敗で止まらなくなる。古いフォルダの掃除は `unregister.ps1` で行う（使用中は残して、その旨を出す）。

## 4. 検討した案

### 4.1 案 A（採る）: 版ごとのフォルダと、登録の向け直し
- 良い点: 使用中のファイルに一度も書かない。再起動なしで、新しいプロセスから新しい版になる。DLL と資源（`resources`）が同じフォルダにあり、版が混ざらない（古い explorer は古い資源を読み続ける）。前例（weasel）がある。道具を選ばない（Inno Setup でも MSI でも、手の `regsvr32` でも同じ）。
- 悪い点: 古い版のフォルダが残る（数 MB。次の更新か再起動で消える）。登録を書き換える瞬間に始まったプロセスが新旧どちらを読むかは、時刻で決まる（どちらでも動くので害はない）。
- 前提: §1.4 の推測（新しいプロセスは新しいパスを読む）。§7 で確かめる。

### 4.2 案 B（採らない）: 決まった名前のまま、改名して置く（`ikari_tip.dll` → `ikari_tip.dll.old`、新しいものを `ikari_tip.dll` に）
- 良い点: 登録のパスが変わらない。開発の過程で通った方法。
- 悪い点: 改名した古いファイルの掃除が要るのは案 A と同じ。同じフォルダの `resources` は上書きになり、古い DLL が新しい資源を読む（キー割り当ての表の形が変わると、古い TIP が壊れうる）。続けて更新すると、`.old` が使用中で消せない・改名できない場合の扱いが要る。案 A より得るものがない。

### 4.3 案 C（採らない）: 置き換えを再起動に予約する（`MOVEFILE_DELAY_UNTIL_REBOOT` だけ、Inno Setup の `restartreplace`、Windows Installer の再起動の置き換え）
- 良い点: インストーラの道具が用意している。Mozc は MSI に任せている。
- 悪い点: **再起動するまで、新しく開いたアプリでも古い版のまま**。サインアウトでは切り替わらない（§1.3 の公式の文書）。再起動を求めるのは、方針の 1 本目（思考を止めない）に反する。Windows Installer に任せると MSI を選ぶことも決めてしまう（ADR 0040 §2.7 では Inno Setup か WiX かを W5 で決める）。

### 4.4 案 D（採らない）: TIP を薄い入口にし、中身を別の DLL か別のプロセスに置く
- 別の DLL（入口が中身を `LoadLibrary`）: 入口も中身も、読み込んだプロセスが持ち続けるので、同じ問題が中身の側に移るだけ。入口を変える更新では元の問題に戻る。
- 別のプロセス（キーごとにプロセスをまたぐ）: ADR 0040 §2.2 の案 S-a（TIP の中で `shell-core` を動かし、キーごとの往復をなくす）を覆す。遅延が増え、エンジンが落ちたときの入力の状態の保ち方（#329）も作り直しになる。
- 重いもの、変わりやすいもの（エンジン、LM、候補窓）はすでに別プロセスにある（ADR 0040、0060 の Q4、0062）。TIP に残るのは `shell-core` と COM の層で、これらは更新のたびに変わる。「薄くすれば更新が要らなくなる」は成り立たない。

### 4.5 MSIX（採らない）
- TIP は全部のアプリのプロセスに読み込まれる in-proc の COM で、HKCR（HKLM）への登録が要る。MSIX のパッケージの COM の登録が、パッケージの外のプロセスの in-proc の読み込みに使えるかは調べていない（推測では使えない。要るなら researcher に一次情報を頼む）。署名が必須で、未署名の配布（#99、ADR 0040 §2.7）と合わない。これだけで採らない理由になる。

## 5. 未署名・0.x の配布（#99）との関係
- 更新は、利用者が Release から新しいインストーラを落として流す（自動の確認は持たない。ADR 0058 と同じ）。インストーラは毎回 SmartScreen の警告を出す見込み（S7。項目 31 で実際の表示を記録する）。DLL の登録そのものに別の警告は出ない見込み（推測）。
- Smart App Control が有効な機械で、未署名の DLL がアプリのプロセスに読み込まれるか（ブロックされるか）は未確認（S7）。更新の方法には影響しないが、配布の前提として残る。
- 版の名前（`tip\<版>\` の `<版>`）は、セマンティックバージョニングの版（`0.3.1` など）。同じ版を入れ直すとき（修復）は、そのフォルダが使用中なら上書きできないので、**中身が同じなら（sha256 で比べる）ファイルを置かず、登録だけし直す**。開発用は版の代わりに日時を使い、毎回別のフォルダにする。
- インストール先: 版ごとのフォルダはどの場所でも成り立つ。TIP は管理者で動くアプリにも読み込まれるので、利用者が書ける場所（`%LOCALAPPDATA%`）に置くと、同じ利用者の権限の低いプログラムが DLL を差し替えられ、それが管理者のアプリに読み込まれる。Program Files（管理者で入れる）を推奨する。本決定は W5 のインストーラの ADR と W6 のレビュー（Q2）。

## 6. 結果
- 更新は再起動なしで終わる。古い版と新しい版が同時に動く時間がある（サインアウトまで）。エンジンと候補窓は 1 つずつなので、TIP の版が混ざっても学習や候補窓がぶつからない見込み（推測。§7 の手順 5 で確かめる）。
- `schema_version` を上げる変更は、以後「サインアウトまで、開いていたアプリはかなだけになる」という利用者に見える費用を持つ。上げる判断は ADR で行う。
- `com/host.rs` の資源の場所（DLL の隣）は、この形のまま本決定にできる（`TODO(W5)` を閉じられる）。
- 古いフォルダの掃除は、インストーラが管理者で行う。エンジン（利用者の権限）は Program Files を消せないので、掃除をエンジンに持たせない。

## 7. スパイク（VM。捨てる手順。main に入れない）
準備: VM のスナップショットを取る。TIP を 2 つビルドする（違いは DbgView に出す版の文字列と、`Hello` の `shell_version` だけ）。管理者の PowerShell で行う。モジュールのパスを見るのは `Get-Process | ForEach-Object { $_.Modules } | Where-Object FileName -like '*ikari_tip*'`（`tasklist /m` はパスを出さない）。
1. 版 1 を `C:\Program Files\ikari-spike\tip\1\` に置いて `regsvr32`。入力方式に足し、Notepad と Edge で打つ。explorer と WindowsTerminal が版 1 を持つことを見る。
2. 版 2 を `…\tip\2\` に置き、**解除せずに**版 2 の `regsvr32`。`InprocServer32` が版 2 を指すこと、入力方式の一覧と既定の入力方式が変わらないこと（設定の画面と `Get-WinUserLanguageList`）を記録する。
3. すでに開いている Notepad で打つ → 版 1 のまま動くか。新しく開いた Notepad で打つ → 版 2 か（DbgView の版の文字列とモジュールのパス）。explorer を再起動 → 版 2 になるか。
4. サインアウト → サインイン。すべてのプロセスが版 2 になり、`tip\1\` を消せるか。
5. 版 1 と版 2 が同時に動く状態で、同じエンジンに両方から変換する。学習が両方から保存されるか、エンジンのログに 2 つの `shell_version` が出るか（出ないなら、出す必要があるかを決める）。
6. `tip\1\` が使用中の間に、`MoveFileEx(…, NULL, MOVEFILE_DELAY_UNTIL_REBOOT)` で削除を予約 → サインアウトだけでは残ること、再起動で消えることを見る。
7. 版の食い違い: `schema_version` を 2 にしたエンジンを入れ、版 1 の TIP が `SCHEMA_MISMATCH` でかなだけになり、固まらないことを見る。古いエンジンを止めるのに `Shutdown` が通ることも見る。
8. 削除: 使用中のまま `DllUnregisterServer` → 開いているアプリで打つとどうなるか（落ちないか、打てるか）、新しく開いたアプリに ikari が出ないこと、入力方式の一覧から消えること。
9. 改名（案 B の前提。§1.1 の記録を確かめる）: 使用中の DLL を `Rename-Item` できるか。
- 記録するもの: 各手順の結果（○△×）、DbgView の行、モジュールのパス、エラーの文。結果は #377 と項目 29 に書き、この ADR の §1.4 を事実に直す。

## 8. 実装の進め方（承認のあと）
1. スパイク（§7）。結果で本決定にする。
2. `windows/dev/register.ps1`・`unregister.ps1` を版ごとのフォルダに替える（開発用。#377 の完了の条件）。項目 29 の手順を書き直す。
3. W5 のインストーラの ADR（道具の選択、インストール先、データの束との関係）で、§3 の 4・5・7 を手順にする。
4. TIP のパイプの相手の確認（インストール先のパス）を足すときは、エンジンの決まったパス（§3 の 5）と照らす。

## 9. ユーザーに聞くこと
- **Q1. 更新の方法**: 版ごとのフォルダに置いて登録を向け直す（案 A。再起動なし。サインアウトで全部切り替わる）。**推奨: 案 A**。案 C（再起動で置き換え）は再起動を求めるので採らない。
- **Q2. インストール先**: Program Files（管理者で入れる。DLL を権限の低いプログラムに差し替えられない）か、`%LOCALAPPDATA%`（管理者なしで入れられる可能性。ただし HKCU の登録で TSF が読むかが未確認で、差し替えの心配がある）。**推奨: Program Files**。W5 のインストーラの ADR で本決定にする。
- **Q3. 削除のときのユーザーデータ**（学習、設定、ユーザー辞書）: **推奨: 既定は残す。削除の画面に「データも消す」の選択を置く（既定はオフ）**。
- **Q4. 更新の最後の案内**: **推奨: 「サインアウトすると全部のアプリで新しい版になります」の文だけ**（作業中の利用者を止めない）。「今すぐサインアウト」のボタンも置く案もある。

## 10. 参考
- #377、#331（2026-11-14 の実機の結果）、#99、`docs/windows-real-device-check.md` の項目 29・30・31
- [ADR 0040](0040-windows-feasibility.md) §2.2・§2.7・§6（S7、S9）・§7（W5、W6）、[ADR 0058](0058-offline-resource-install.md)、[ADR 0060](0060-user-selectable-lm-model.md)、[ADR 0062](0062-windows-candidate-window.md)
- weasel の `output/install.nsi`（rime/weasel）、Mozc の `src/win32/installer/installer_oss_64bit.wxs`（google/mozc）、azooKey-Windows の `installer/Installer.iss`（fkunn1326/azooKey-Windows）、Microsoft の Windows-classic-samples（SampleIME）。いずれも 2026-11-14 の既定のブランチ
- Microsoft Learn: `MoveFileExW`（`MOVEFILE_DELAY_UNTIL_REBOOT`）、Windows Installer の「System Reboots」「Replacing Existing Files」
