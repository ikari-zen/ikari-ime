//! Windows のインストーラ（`windows/installer/ikari.iss`）の静的な検査（ADR 0067 §3・§4、0068 §6.2、0070 §4、0073 §4、0080 §6・§7.3・§7.4・§11）。
//! Inno Setup は Mac で動かせないので、守ることを文字列と設定で確かめる。実際の動き（ACL、登録、規則、自動起動、
//! 止める側、エラー報告の除外、同梱のデータ、通常版とオフライン版）は、実機の手順書（`docs/windows-real-device-check.md` の項目 47〜）で確かめる。
//!
//! 見ること:
//!   - Inno Setup の下限の版（6.6.0）を、コンパイルのときに要求していること
//!   - `/DEdition=normal|offline` の分岐（既定 `normal`、`offline` の出力名の `-offline`、不正な値を `#error` で断ること。ADR 0080 §7.3・§7.4）
//!   - `[Setup]` の安全に関わる値（管理者、固定のインストール先、使用中のアプリを閉じない、ログを出さない、64 ビット、版の接尾辞つきの出力名）
//!   - 利用者ごとの場所・利用者が書ける一時の場所・シェル・PowerShell に触れていないこと
//!   - 外のプログラムの起動が 1 か所（`RunTrusted`）で、相手が system32 の道具か helper だけであること
//!   - DLL の登録に `regserver` の印、置き換えに `restartreplace` の印を使っていないこと
//!   - `[Files]` の置き先が、インストール先か作業用のフォルダだけで、インストール先の項目が検査（Check）を通ること
//!   - `ikari-data.exe` が `[Files]` の `#if Edition == "normal"` の中にだけあり、`[Code]`（ファイアウォールの規則など）に無いこと
//!   - 同梱のデータ（`{app}\data\dict\`、`{app}\data\models\`、`{app}\data\NOTICE.txt`、`{app}\data\licenses\`）と
//!     `{app}\THIRD_PARTY_NOTICES.md` が、`xtask/src/scan/macos.rs` と `ikari_resource::ZENZ_SMALL_FILE_NAME` の表と過不足なく一致すること
//!   - インストールと削除の手順の順番（ADR 0067 §3.2）。インストールの手順は、失敗したら後を打ち切ること
//!   - 置く前の準備（検査 → エンジンの停止 → 前の版の LM の部品の掃除）の順番と、準備が作業用のフォルダに
//!     道具を置いたあとに始まること（Inno は、ファイルを置く前の大きさの見積もりでも Check を呼ぶ）
//!   - 削除の情報の登録（`CreateUninstallRegKey`）が、置いたあとの検査に通ったときだけであること
//!   - ファイアウォールとエラー報告の対象の exe が、helper の止める相手と同じ並びで、規則の名前に版が入らないこと
//!   - 自動起動が HKLM の Run であること
//!   - helper の終了コードが、helper の定義と同じ値であること

use std::path::Path;

use ikari_resource::ZENZ_SMALL_FILE_NAME;

use crate::data::run::{Failure, log};

const ISS: &str = "windows/installer/ikari.iss";
const HELPER_LIB: &str = "windows/installer/helper/src/lib.rs";
const HELPER_STOP: &str = "windows/installer/helper/src/stop.rs";
const SCAN_MACOS: &str = "xtask/src/scan/macos.rs";

/// Inno Setup の下限（一時の場所の名前を CSPRNG で作る、SYSTEM のときの SystemTemp。2026-11 の調査）
const MIN_INNO_GUARD: &str = "#if Ver < EncodeVer(6, 6, 0)";

/// `/DEdition=normal|offline` の分岐（ADR 0080 §7.3、§7.4）。先頭（`[Setup]` より前）にこの並びで書かれていること
const EDITION_GUARD: &[&str] = &[
    "#ifndef Edition",
    "#define Edition \"normal\"",
    "#endif",
    "#if Edition == \"normal\"",
    "#define EditionSuffix \"\"",
    "#elif Edition == \"offline\"",
    "#define EditionSuffix \"-offline\"",
    "#else",
    "#error /DEdition は normal か offline です",
    "#endif",
];
/// `[Files]` の中で許す `#if`（通常版だけの項目。ADR 0080 §7.3）
const EDITION_NORMAL_IF: &str = "#if Edition == \"normal\"";

/// `[Setup]` に必要な値（キーは大文字小文字を区別しない、値は区別する）
const REQUIRED_SETUP: &[(&str, &str)] = &[
    ("PrivilegesRequired", "admin"),
    ("DefaultDirName", r"{commonpf64}\ikari-ime"),
    ("DisableDirPage", "yes"),
    ("UsePreviousAppDir", "no"),
    ("CloseApplications", "no"),
    ("RestartApplications", "no"),
    ("SetupLogging", "no"),
    ("ArchitecturesAllowed", "x64compatible"),
    ("ArchitecturesInstallIn64BitMode", "x64compatible"),
    // 検査に落ちた場所を指す削除の入口を作らない（置いたあとの検査の結果を返す関数）
    ("CreateUninstallRegKey", "UninstallEntryAllowed"),
    (
        "OutputBaseFilename",
        "ikari-ime-{#AppVersion}-windows-x64-setup{#EditionSuffix}",
    ),
];
/// `[Setup]` に書いてはいけないキー（利用者ごとの導入への切り替え）
const FORBIDDEN_SETUP: &[&str] = &["PrivilegesRequiredOverridesAllowed"];

/// どこにも書いてはいけない語（コメントを除く。大文字小文字を区別しない）
const FORBIDDEN_WORDS: &[(&str, &str)] = &[
    ("{tmp}", "利用者の TEMP の下の一時の場所"),
    ("ExtractTemporaryFile", "一時の場所への展開"),
    ("{localappdata}", "利用者ごとの場所"),
    ("{userappdata}", "利用者ごとの場所"),
    ("{userdocs}", "利用者ごとの場所"),
    ("{userdesktop}", "利用者ごとの場所"),
    ("{userpf}", "利用者ごとの場所"),
    ("{%", "環境変数"),
    ("GetEnv(", "環境変数"),
    ("HKCU", "利用者ごとのレジストリ"),
    ("HKEY_CURRENT_USER", "利用者ごとのレジストリ"),
    ("HKA", "利用者ごとのレジストリになりうる"),
    (
        "powershell",
        "PowerShell（利用者の場所からモジュールを読みうる）",
    ),
    ("cmd.exe", "シェル"),
    ("ShellExec", "関連づけの起動"),
];
/// `[Files]` の印で使ってはいけないもの
const FORBIDDEN_FILE_FLAGS: &[(&str, &str)] = &[
    (
        "regserver",
        "登録は明示の regsvr32（Program Files の DLL）で行う",
    ),
    (
        "restartreplace",
        "使用中の DLL の置き換えを再起動に回さない（ADR 0063 案 C）",
    ),
    (
        "deleteafterinstall",
        "作業用のフォルダは DeinitializeSetup で消す",
    ),
];
/// `[Files]` の置き先として許すものの前置き
const ALLOWED_DEST: &[&str] = &["{app}", "{code:SetupWorkDir}"];
/// インストール先の項目が通す Check
const APP_CHECKS: &[&str] = &["AppDirReady", "TipNeedsCopy"];
/// `RunTrusted` の最初の引数として許すもの
const TRUSTED_EXES: &[&str] = &["SysTool(", "WorkHelper", "AppHelper", "Helper"];
/// インストールの手順の順（ADR 0067 §3.2 の 3〜7）
const INSTALL_ORDER: (&str, &[&str]) = (
    "FinishInstall",
    &[
        "VerifyAppDir",
        "RegisterTip",
        "AddFirewallRules",
        "AddAutostart",
        "ExcludeFromErrorReporting",
        "RemoveOldTipVersions",
    ],
);
/// 置く前の準備の順（最初の `{app}` の項目の Check。ADR 0067 §3.2 の 2・3）。前の版の LM の部品は、
/// 検査とエンジンの停止のあとに消す（`[InstallDelete]` は検査より前に動くので使わない）
const PREPARE_ORDER: (&str, &[&str]) = (
    "AppDirReady",
    &["CheckAppDir", "StopEngines", "RemoveStaleLmLibraries"],
);
/// 作業用のフォルダに道具を置いたときに呼ぶ手続き（`[Files]` の AfterInstall）と、準備の前に見る印
const WORK_HELPER_PLACED: &str = "WorkHelperPlaced";
const WORK_HELPER_GUARD: &str = "if not WorkHelperReady then";
/// 削除の手順の順（ADR 0067 §3.2 の逆順）
const UNINSTALL_ORDER: (&str, &[&str]) = (
    "UninstallInOrder",
    &[
        "RemoveAutostart",
        "RemoveFirewallRules",
        "IncludeInErrorReporting",
        "UnregisterTip",
        "StopEngines",
        "DeleteCageProfile",
    ],
);
/// helper の終了コードの名前（lib.rs の `exit` の定数）と、ikari.iss の定数の名前
const EXIT_CODES: &[(&str, &str)] = &[
    ("OK", "HelperOk"),
    ("UNTRUSTED", "HelperUntrusted"),
    ("REMAINING", "HelperRemaining"),
];

pub fn lint(root: &Path) -> Result<(), Failure> {
    log(
        "Windows のインストーラの静的な検査（windows/installer/ikari.iss。ADR 0067 §3・§4、0080 §6・§7.3）",
    );
    let iss = read(root, ISS)?;
    let lib = read(root, HELPER_LIB)?;
    let stop = read(root, HELPER_STOP)?;
    let scan_macos = read(root, SCAN_MACOS)?;
    let problems = problems(&iss, &lib, &stop, &scan_macos);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            problems
                .iter()
                .map(|p| format!("{ISS}: {p}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }
}

fn read(root: &Path, relative: &str) -> Result<String, Failure> {
    std::fs::read_to_string(root.join(relative))
        .map_err(|e| Failure::new(format!("{relative} を読めません: {e}")))
}

/// 1 行（コメントを除いた中身）と、その行の節の名前
struct Line<'a> {
    section: String,
    text: &'a str,
}

/// `[Code]` 以外の、コメント（`;` で始まる行）を除いた行
fn lines(iss: &str) -> Vec<Line<'_>> {
    let mut section = String::new();
    let mut out = Vec::new();
    for raw in iss.lines() {
        let text = raw.trim();
        if text.is_empty() || text.starts_with(';') || text.starts_with("//") {
            continue;
        }
        if text.starts_with('[') && text.ends_with(']') && !text.contains(' ') {
            section = text[1..text.len() - 1].to_ascii_lowercase();
            continue;
        }
        if section == "code" {
            // [Code] は注記の形が違うので `code_body` で読む
            continue;
        }
        out.push(Line {
            section: section.clone(),
            text,
        });
    }
    out
}

/// `[Code]` の複数行の注記（`{` で始まり `}` で終わる）を取り除いた本文
fn code_body(iss: &str) -> String {
    let mut in_code = false;
    let mut depth = 0usize;
    let mut out = String::new();
    for raw in iss.lines() {
        let text = raw.trim();
        if text.starts_with('[') && text.ends_with(']') && !text.contains(' ') {
            in_code = text.eq_ignore_ascii_case("[Code]");
            continue;
        }
        if !in_code {
            continue;
        }
        let mut kept = String::new();
        let mut chars = raw.chars().peekable();
        let mut in_string = false;
        while let Some(c) = chars.next() {
            if depth == 0 && c == '\'' {
                in_string = !in_string;
            }
            if !in_string {
                if c == '{' && chars.peek() != Some(&'#') {
                    depth += 1;
                    continue;
                }
                if c == '}' && depth > 0 {
                    depth -= 1;
                    continue;
                }
                if depth == 0 && c == '/' && chars.peek() == Some(&'/') {
                    break;
                }
            }
            if depth == 0 {
                kept.push(c);
            }
        }
        out.push_str(&kept);
        out.push('\n');
    }
    out
}

/// `[Code]` の `{ … }` の注記の中に `{` があると、Inno Setup の Pascal は注記を入れ子にせず、最初の `}` で閉じる。
/// 残りが構文エラーになる（`{app}` を書いた注記で、ISCC が落ちた）。その行番号（1 始まり）を返す
fn nested_brace_comment_lines(iss: &str) -> Vec<usize> {
    let mut in_code = false;
    let mut in_comment = false;
    let mut found = Vec::new();
    for (index, raw) in iss.lines().enumerate() {
        let text = raw.trim();
        if text.starts_with('[') && text.ends_with(']') && !text.contains(' ') {
            in_code = text.eq_ignore_ascii_case("[Code]");
            in_comment = false;
            continue;
        }
        if !in_code {
            continue;
        }
        let mut in_string = false;
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if !in_comment && c == '\'' {
                in_string = !in_string;
            }
            if in_string {
                continue;
            }
            if !in_comment && c == '/' && chars.peek() == Some(&'/') {
                break;
            }
            if c == '{' && chars.peek() != Some(&'#') {
                if in_comment {
                    found.push(index + 1);
                    break;
                }
                in_comment = true;
            } else if c == '}' && in_comment {
                in_comment = false;
            }
        }
    }
    found
}

fn problems(iss: &str, helper_lib: &str, helper_stop: &str, scan_macos: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lines = lines(iss);

    for line in nested_brace_comment_lines(iss) {
        found.push(format!(
            "[Code] の {line} 行目: `{{ … }}` の注記の中に `{{` がある（Inno Setup は入れ子にせず最初の `}}` で閉じて構文エラーになる。`{{app}}` などを書かない）"
        ));
    }

    if !iss.lines().any(|l| l.trim() == MIN_INNO_GUARD) {
        found.push(format!(
            "Inno Setup の下限の検査（`{MIN_INNO_GUARD}` と #error）がありません"
        ));
    }

    let header: Vec<&str> = lines
        .iter()
        .filter(|l| l.section.is_empty())
        .map(|l| l.text)
        .collect();
    if !header
        .windows(EDITION_GUARD.len())
        .any(|w| w == EDITION_GUARD)
    {
        found.push(
            "/DEdition=normal|offline の検査と EditionSuffix の定義がありません（ADR 0080 §7.3）"
                .into(),
        );
    }

    // [Setup]
    let setup: Vec<(String, String)> = lines
        .iter()
        .filter(|l| l.section == "setup")
        .filter_map(|l| l.text.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    for (key, value) in REQUIRED_SETUP {
        match setup.iter().find(|(k, _)| k == &key.to_ascii_lowercase()) {
            Some((_, v)) if v == value => {}
            Some((_, v)) => found.push(format!("[Setup] {key} は {value}（今は {v}）")),
            None => found.push(format!("[Setup] に {key}={value} がありません")),
        }
    }
    for key in FORBIDDEN_SETUP {
        if setup.iter().any(|(k, _)| k == &key.to_ascii_lowercase()) {
            found.push(format!("[Setup] に {key} を書かない"));
        }
    }

    // 語（[Code] は注記を除いた本文）
    let code = code_body(iss);
    for text in lines.iter().map(|l| l.text).chain(code.lines()) {
        let lower = text.to_ascii_lowercase();
        for (word, why) in FORBIDDEN_WORDS {
            let w = word.to_ascii_lowercase();
            let hit = if word.chars().all(|c| c.is_ascii_alphabetic()) {
                contains_word(&lower, &w)
            } else {
                lower.contains(&w)
            };
            if hit {
                found.push(format!("`{word}` を使わない（{why}）: {}", text.trim()));
            }
        }
    }

    // [Files]
    check_files(&lines, scan_macos, &mut found);
    for text in lines
        .iter()
        .filter(|l| l.section != "files")
        .map(|l| l.text)
        .chain(code.lines())
    {
        if text.to_ascii_lowercase().contains(STOP_DATA_EXE) {
            found.push(format!(
                "`{STOP_DATA_EXE}` は [Files] の通常版の項目のほかに書かない（ファイアウォールの規則もかけない。ADR 0076、0080 §7.3）: {}",
                text.trim()
            ));
        }
    }

    // [Code]
    let exec_count = code.matches("Exec(").count();
    if exec_count != 1 {
        found.push(format!(
            "Exec( は RunTrusted の中の 1 か所だけ（今は {exec_count}）"
        ));
    }
    for call in code.match_indices("RunTrusted(").map(|(i, _)| &code[i..]) {
        let first = call["RunTrusted(".len()..].trim_start();
        if first.starts_with("Exe,") || first.starts_with("Exe, ") {
            continue; // 定義そのもの
        }
        if !TRUSTED_EXES.iter().any(|t| first.starts_with(t)) {
            let shown: String = call.chars().take(60).collect();
            found.push(format!(
                "RunTrusted の相手は {} だけ: {shown}",
                TRUSTED_EXES.join("、")
            ));
        }
    }
    for call in code.match_indices("StepOk(").map(|(i, _)| &code[i..]) {
        let args = &call["StepOk(".len()..];
        if args.trim_start().starts_with("Ok:") {
            continue; // 定義そのもの
        }
        let first = first_argument(args);
        if first.contains("RunTrusted(") || first.contains("Exec(") {
            found.push(
                "StepOk の第 1 引数の中で RunTrusted( や Exec( を呼ばない（var 引数 Code が実行前の値で渡り、失敗を見逃す）。先に Result := RunTrusted(…, Code); と代入してから StepOk(Result, Code, …) へ"
                    .to_string(),
            );
        }
    }
    for (procedure, order) in [INSTALL_ORDER, PREPARE_ORDER, UNINSTALL_ORDER] {
        match procedure_body(&code, procedure) {
            None => found.push(format!("procedure {procedure} がありません")),
            Some(body) => {
                if let Err(p) = in_order(&body, order) {
                    found.push(format!("{procedure} の順: {p}"));
                }
            }
        }
    }
    if !procedure_body(&code, PREPARE_ORDER.0).is_some_and(|b| b.contains(WORK_HELPER_GUARD)) {
        found.push(format!(
            "{} は、道具を置く前に呼ばれたときは何もしない（`{WORK_HELPER_GUARD}`）",
            PREPARE_ORDER.0
        ));
    }
    // インストールの手順は、最後のもの以外、失敗したら後を打ち切る（`if not 手順 then`）
    if let Some(body) = procedure_body(&code, INSTALL_ORDER.0) {
        let (_, steps) = INSTALL_ORDER;
        for step in &steps[..steps.len() - 1] {
            if !body.contains(&format!("if not {step} then")) {
                found.push(format!(
                    "{} は {step} が失敗したら後を打ち切る（`if not {step} then`）",
                    INSTALL_ORDER.0
                ));
            }
        }
    }
    if !code.contains("RegWriteStringValue(HKLM64, RunKey, RunValue") {
        found.push("自動起動は HKLM64 の RunKey に書く（ADR 0067 §4.1）".into());
    }
    if !code.contains(r"RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';") {
        found.push(r"RunKey は Software\Microsoft\Windows\CurrentVersion\Run".into());
    }
    match code
        .lines()
        .find(|l| l.trim_start().starts_with("FirewallRulePrefix ="))
    {
        None => found.push("FirewallRulePrefix がありません".into()),
        Some(l) if l.contains("AppVersion") => {
            found.push("ファイアウォールの規則の名前に版を入れない".into())
        }
        Some(_) => {}
    }

    // exe の並び（helper の止める相手と同じ）
    let blocked = case_literals(&code, "BlockedExe");
    let expected = helper_blocked_exes(helper_stop);
    if expected.is_empty() {
        found.push(format!(
            "{HELPER_STOP} の ENGINE_EXE と BLOCKED_CHILD_EXES を読めません"
        ));
    } else if blocked != expected {
        found.push(format!(
            "BlockedExe の並び {blocked:?} が helper の送信ブロックの一覧 {expected:?} と違う"
        ));
    }
    // 止める相手は、送信ブロックの子に、取得の子（ikari-data.exe）を足したもの
    let stop = helper_stop_exes(helper_stop);
    let mut want_stop: Vec<String> = expected.iter().skip(1).cloned().collect();
    want_stop.push(STOP_DATA_EXE.to_string());
    if stop != want_stop {
        found.push(format!(
            "STOP_CHILD_EXES {stop:?} が送信ブロックの子と {STOP_DATA_EXE} の並び {want_stop:?} と違う"
        ));
    }

    // 終了コード
    for (helper_name, iss_name) in EXIT_CODES {
        let want = const_value(helper_lib, &format!("pub const {helper_name}: u8 ="));
        let have = const_value(&code, &format!("{iss_name} ="));
        if want.is_none() || want != have {
            found.push(format!(
                "{iss_name}（{have:?}）が helper の exit::{helper_name}（{want:?}）と違う"
            ));
        }
    }
    found
}

/// ASCII の語の区切りで含むか
/// 呼び出しの括弧の中の、最初の引数（最上位のカンマまで。文字列の中は読み飛ばす）
fn first_argument(args: &str) -> &str {
    let (mut depth, mut quoted) = (0usize, false);
    for (i, c) in args.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            _ if quoted => {}
            '(' => depth += 1,
            ')' if depth == 0 => return &args[..i],
            ')' => depth -= 1,
            ',' if depth == 0 => return &args[..i],
            _ => {}
        }
    }
    args
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + word.len()..].chars().next();
        let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        boundary(before) && boundary(after)
    })
}

/// `Name: "value"; Name2: value2` の形を (小文字の名前, 値) に
fn parameters(text: &str) -> Vec<(String, String)> {
    text.split(';')
        .filter_map(|p| p.split_once(':'))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_lowercase(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> &'a str {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// `[Files]` に入れる同梱データ（`{app}\data\`）の 1 項目（ADR 0080 §3.1、§4、§6、§11）
struct DataEntry {
    source: String,
    dest: &'static str,
    optional: bool,
    recurse: bool,
}

/// `xtask/src/scan/macos.rs` と `ikari_resource` の表から、同梱データの期待の一覧を作る（OS ごとに一覧を重ねて持たない）
fn expected_data_files(scan_macos: &str, found: &mut Vec<String>) -> Vec<DataEntry> {
    let dict_required = const_list(scan_macos, "DICT_REQUIRED");
    let dict_optional = const_list(scan_macos, "DICT_OPTIONAL");
    if dict_required.is_empty() || dict_optional.is_empty() {
        found.push(format!(
            "{SCAN_MACOS} の DICT_REQUIRED と DICT_OPTIONAL を読めません"
        ));
        return Vec::new();
    }
    let mut out = Vec::new();
    for name in dict_required {
        out.push(DataEntry {
            source: format!(r"{{#StageDir}}\data\dict\{name}"),
            dest: r"{app}\data\dict",
            optional: false,
            recurse: false,
        });
    }
    for name in dict_optional {
        out.push(DataEntry {
            source: format!(r"{{#StageDir}}\data\dict\{name}"),
            dest: r"{app}\data\dict",
            optional: true,
            recurse: false,
        });
    }
    out.push(DataEntry {
        source: format!(r"{{#StageDir}}\data\models\{ZENZ_SMALL_FILE_NAME}"),
        dest: r"{app}\data\models",
        optional: false,
        recurse: false,
    });
    out.push(DataEntry {
        source: r"{#StageDir}\data\NOTICE.txt".into(),
        dest: r"{app}\data",
        optional: false,
        recurse: false,
    });
    out.push(DataEntry {
        source: r"{#StageDir}\data\licenses\*".into(),
        dest: r"{app}\data\licenses",
        optional: false,
        recurse: true,
    });
    out
}

fn check_files(lines: &[Line<'_>], scan_macos: &str, found: &mut Vec<String>) {
    let expected_data = expected_data_files(scan_macos, found);
    let mut file_guard: Option<&str> = None;
    let mut data_exe_count = 0usize;
    let mut third_party_notices_count = 0usize;
    let mut seen_data_sources: Vec<String> = Vec::new();

    for line in lines.iter().filter(|l| l.section == "files") {
        if line.text.starts_with('#') {
            if line.text == EDITION_NORMAL_IF && file_guard.is_none() {
                file_guard = Some(EDITION_NORMAL_IF);
            } else if line.text == "#endif" && file_guard.is_some() {
                file_guard = None;
            } else {
                found.push(format!(
                    "[Files] の条件分岐は `{EDITION_NORMAL_IF}` … `#endif` だけ: {}",
                    line.text
                ));
            }
            continue;
        }
        let params = parameters(line.text);
        let source = param(&params, "source");
        let dest = param(&params, "destdir");
        let check = param(&params, "check");
        let flags = param(&params, "flags").to_ascii_lowercase();
        let flag_list: Vec<&str> = flags.split_whitespace().collect();

        for (flag, why) in FORBIDDEN_FILE_FLAGS {
            if flag_list.contains(flag) {
                found.push(format!(
                    "[Files] の印 {flag} を使わない（{why}）: {}",
                    line.text
                ));
            }
        }
        if !ALLOWED_DEST.iter().any(|d| dest.starts_with(d)) {
            found.push(format!(
                "[Files] の置き先は {} だけ: {}",
                ALLOWED_DEST.join(" か "),
                line.text
            ));
        }
        if dest.starts_with("{code:SetupWorkDir}")
            && !params
                .iter()
                .any(|(k, v)| k == "afterinstall" && v == WORK_HELPER_PLACED)
        {
            found.push(format!(
                "[Files] の作業用のフォルダの項目は AfterInstall: {WORK_HELPER_PLACED} を付ける（準備はそのあと）: {}",
                line.text
            ));
        }
        if dest.starts_with("{app}") && !APP_CHECKS.contains(&check) {
            found.push(format!(
                "[Files] のインストール先の項目は Check: {} を通す: {}",
                APP_CHECKS.join(" か "),
                line.text
            ));
        }

        let is_data_exe = source.eq_ignore_ascii_case(&format!(r"{{#StageDir}}\{STOP_DATA_EXE}"))
            || line.text.to_ascii_lowercase().contains(STOP_DATA_EXE);
        if is_data_exe {
            data_exe_count += 1;
            if file_guard != Some(EDITION_NORMAL_IF) {
                found.push(format!(
                    "[Files] の {STOP_DATA_EXE} は `{EDITION_NORMAL_IF}` の中にだけ置く（オフライン版には入れない。ADR 0080 §7.3）: {}",
                    line.text
                ));
            }
            if source != format!(r"{{#StageDir}}\{STOP_DATA_EXE}") || dest != "{app}" {
                found.push(format!(
                    "[Files] の {STOP_DATA_EXE} は Source: \"{{#StageDir}}\\{STOP_DATA_EXE}\"; DestDir: \"{{app}}\": {}",
                    line.text
                ));
            }
            if flag_list.contains(&"skipifsourcedoesntexist") {
                found.push(format!(
                    "[Files] の {STOP_DATA_EXE} に skipifsourcedoesntexist を付けない（通常版では必須）: {}",
                    line.text
                ));
            }
        } else if file_guard.is_some() {
            found.push(format!(
                "[Files] の `{EDITION_NORMAL_IF}` の中に置くのは {STOP_DATA_EXE} だけ（同梱のデータなどは両方の版に入れる。ADR 0080 §7.3）: {}",
                line.text
            ));
        }

        if source.eq_ignore_ascii_case(r"{#StageDir}\THIRD_PARTY_NOTICES.md") {
            third_party_notices_count += 1;
            if dest != "{app}" {
                found.push(format!(
                    "[Files] の THIRD_PARTY_NOTICES.md の DestDir は {{app}}（今は {dest}）"
                ));
            }
            if flag_list.contains(&"skipifsourcedoesntexist") {
                found.push(
                    "[Files] の THIRD_PARTY_NOTICES.md に skipifsourcedoesntexist を付けない（必須。ADR 0080 §4）"
                        .into(),
                );
            }
        }

        let is_bundled_data = source.starts_with(r"{#StageDir}\data\")
            || source == r"{#StageDir}\data"
            || dest == r"{app}\data"
            || dest.starts_with(r"{app}\data\");
        if is_bundled_data {
            if check != "AppDirReady" {
                found.push(format!(
                    "[Files] の同梱データの項目は Check: AppDirReady を通す: {}",
                    line.text
                ));
            }
            seen_data_sources.push(source.to_string());
            match expected_data.iter().find(|e| e.source == source) {
                None => found.push(format!(
                    "[Files] の同梱データに表にない項目があります（一覧は {SCAN_MACOS} と resource/src/models.rs）: {}",
                    line.text
                )),
                Some(expected) => {
                    if dest != expected.dest {
                        found.push(format!(
                            "[Files] の {} の DestDir は {}（今は {dest}）",
                            expected.source, expected.dest
                        ));
                    }
                    let has_opt = flag_list.contains(&"skipifsourcedoesntexist");
                    if has_opt != expected.optional {
                        if expected.optional {
                            found.push(format!(
                                "[Files] の {} は任意の辞書なので skipifsourcedoesntexist を付ける",
                                expected.source
                            ));
                        } else {
                            found.push(format!(
                                "[Files] の {} は必須なので skipifsourcedoesntexist を付けない",
                                expected.source
                            ));
                        }
                    }
                    let has_recurse = flag_list.contains(&"recursesubdirs");
                    if has_recurse != expected.recurse {
                        if expected.recurse {
                            found.push(format!(
                                "[Files] の {} には recursesubdirs を付ける",
                                expected.source
                            ));
                        } else {
                            found.push(format!(
                                "[Files] の {} に recursesubdirs を付けない",
                                expected.source
                            ));
                        }
                    }
                }
            }
        }
    }

    if file_guard.is_some() {
        found.push(format!(
            "[Files] の `{EDITION_NORMAL_IF}` が `#endif` で閉じられていません"
        ));
    }
    if data_exe_count != 1 {
        found.push(format!(
            "[Files] に通常版の `{STOP_DATA_EXE}` の項目がちょうど 1 つ要ります（今は {data_exe_count}）"
        ));
    }
    if third_party_notices_count == 0 {
        found.push(
            "[Files] に `{#StageDir}\\THIRD_PARTY_NOTICES.md`（DestDir: \"{app}\"）がありません（ADR 0080 §4）"
                .into(),
        );
    } else if third_party_notices_count > 1 {
        found
            .push("[Files] に `{#StageDir}\\THIRD_PARTY_NOTICES.md` の項目が重複しています".into());
    }
    for expected in &expected_data {
        let count = seen_data_sources
            .iter()
            .filter(|s| s.as_str() == expected.source)
            .count();
        if count == 0 {
            found.push(format!(
                "[Files] に同梱データの項目 {}（DestDir: \"{}\"）がありません（ADR 0080 §6）",
                expected.source, expected.dest
            ));
        } else if count > 1 {
            found.push(format!(
                "[Files] に同梱データの項目 {} が重複しています",
                expected.source
            ));
        }
    }
}

/// `procedure Name;` か `function Name: …;` から `end;`（行頭）までの本文
fn procedure_body(code: &str, name: &str) -> Option<String> {
    let (start, head_len) = [format!("procedure {name};"), format!("function {name}:")]
        .iter()
        .find_map(|head| code.find(head.as_str()).map(|i| (i, head.len())))?;
    let rest = &code[start + head_len..];
    let end = rest.find("\nend;").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// 本文の中で、名前が `order` の順に（それぞれ 1 回以上）出てくるか
fn in_order(body: &str, order: &[&str]) -> Result<(), String> {
    let mut last = 0usize;
    let mut last_name = "";
    for name in order {
        let Some(at) = body.match_indices(name).map(|(i, _)| i).find(|&i| {
            let after = body[i + name.len()..].chars().next();
            !after.is_some_and(|c| c.is_ascii_alphanumeric())
        }) else {
            return Err(format!("{name} がありません"));
        };
        if at < last {
            return Err(format!("{name} は {last_name} のあと"));
        }
        last = at;
        last_name = name;
    }
    Ok(())
}

/// `function Name(…)` の中の `N: Result := '…';` の文字列を順に
fn case_literals(code: &str, function: &str) -> Vec<String> {
    let head = format!("function {function}(");
    let Some(start) = code.find(&head) else {
        return Vec::new();
    };
    let rest = &code[start..];
    let end = rest.find("\nend;").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter_map(|l| {
            let (label, value) = l.trim().split_once(": Result := '")?;
            label.parse::<u32>().ok()?;
            Some(value.split('\'').next()?.to_string())
        })
        .collect()
}

/// 取得の子。停止の対象にだけ入れる（通信するので、送信ブロックには入れない。ADR 0076）
const STOP_DATA_EXE: &str = "ikari-data.exe";

/// 文字列リテラル（`r"…"` と `"…"`）
fn literal(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches('r');
    Some(s.strip_prefix('"')?.split('"').next()?.to_string())
}

/// `pub const <name>: &[&str] = &[` から `];` までの文字列リテラルを順に（rustfmt が複数行に折ることがある）
fn const_list(stop: &str, name: &str) -> Vec<String> {
    let start_text = format!("pub const {name}: &[&str] = &[");
    let Some(start) = stop.find(&start_text) else {
        return Vec::new();
    };
    let rest = &stop[start + start_text.len()..];
    let inner = rest.split("];").next().unwrap_or_default();
    inner.split(',').filter_map(literal).collect()
}

/// helper の `ENGINE_EXE` と `BLOCKED_CHILD_EXES` を順に（`BlockedExe` と同じ並び）
fn helper_blocked_exes(stop: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stop.lines() {
        if let Some(rest) = line.trim().strip_prefix("pub const ENGINE_EXE: &str =") {
            out.insert(0, literal(rest).unwrap_or_default());
        }
    }
    out.extend(const_list(stop, "BLOCKED_CHILD_EXES"));
    out.retain(|s| !s.is_empty());
    out
}

/// helper の `STOP_CHILD_EXES`（エンジンのあとに終わらせる子）
fn helper_stop_exes(stop: &str) -> Vec<String> {
    const_list(stop, "STOP_CHILD_EXES")
}

/// `prefix` のあとの数（`;` まで）
fn const_value(text: &str, prefix: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let rest = l.trim().strip_prefix(prefix)?;
        rest.trim().trim_end_matches(';').trim().parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_file(relative: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        // Windows の checkout は改行が CRLF になる（core.autocrlf）。置き換える文字列は LF で書くので揃える
        std::fs::read_to_string(root.join(relative))
            .unwrap()
            .replace("\r\n", "\n")
    }

    #[test]
    fn the_real_installer_passes() {
        let problems = problems(
            &repo_file(ISS),
            &repo_file(HELPER_LIB),
            &repo_file(HELPER_STOP),
            &repo_file(SCAN_MACOS),
        );
        assert!(problems.is_empty(), "{problems:#?}");
    }

    fn with(replace: &str, by: &str) -> Vec<String> {
        let iss = repo_file(ISS);
        assert!(iss.contains(replace), "{replace}");
        problems(
            &iss.replacen(replace, by, 1),
            &repo_file(HELPER_LIB),
            &repo_file(HELPER_STOP),
            &repo_file(SCAN_MACOS),
        )
    }

    fn has(problems: &[String], needle: &str) -> bool {
        problems.iter().any(|p| p.contains(needle))
    }

    #[test]
    fn a_brace_inside_a_brace_comment_is_found() {
        let bad = "[Code]\n{ 先頭の {app} の項目 }\nprocedure X;\nbegin\nend;\n";
        assert_eq!(nested_brace_comment_lines(bad), vec![2]);
        let ok = "[Code]\n{ 先頭の app の項目 }\nprocedure X;\nbegin\n  { 注記 }\nend;\n";
        assert!(nested_brace_comment_lines(ok).is_empty());
        let in_string = "[Code]\nprocedure X;\nbegin\n  Y('{app}');\nend;\n";
        assert!(nested_brace_comment_lines(in_string).is_empty());
    }

    #[test]
    fn setup_values_are_enforced() {
        assert!(has(
            &with("DisableDirPage=yes", "DisableDirPage=no"),
            "DisableDirPage"
        ));
        assert!(has(
            &with("PrivilegesRequired=admin", "PrivilegesRequired=lowest"),
            "PrivilegesRequired"
        ));
        assert!(has(
            &with(
                "PrivilegesRequired=admin",
                "PrivilegesRequired=admin\nPrivilegesRequiredOverridesAllowed=dialog"
            ),
            "PrivilegesRequiredOverridesAllowed"
        ));
        assert!(has(
            &with("SetupLogging=no", "SetupLogging=yes"),
            "SetupLogging"
        ));
        assert!(has(
            &with("CloseApplications=no", "CloseApplications=yes"),
            "CloseApplications"
        ));
        assert!(has(&with(MIN_INNO_GUARD, "#if 0"), "下限"));
    }

    #[test]
    fn user_places_temp_and_shells_are_refused() {
        let line = "procedure Bad;\nbegin\n  RegWriteStringValue(HKCU, 'x', 'y', 'z');\nend;\n\nprocedure FinishInstall;";
        assert!(has(&with("procedure FinishInstall;", line), "HKCU"));
        assert!(has(
            &with(
                "procedure FinishInstall;",
                "procedure Bad;\nbegin\n  ExtractTemporaryFile('a');\nend;\n\nprocedure FinishInstall;"
            ),
            "ExtractTemporaryFile"
        ));
        assert!(has(
            &with(r#"DestDir: "{code:SetupWorkDir}""#, r#"DestDir: "{tmp}""#),
            "{tmp}"
        ));
        assert!(has(
            &with(
                r#"DestDir: "{code:SetupWorkDir}""#,
                r#"DestDir: "{localappdata}\ikari-ime""#
            ),
            "{localappdata}"
        ));
    }

    #[test]
    fn comments_may_mention_forbidden_words() {
        assert!(
            with(
                "[Setup]",
                "; HKCU と {tmp} と powershell には触れない\n[Setup]"
            )
            .is_empty()
        );
    }

    #[test]
    fn file_flags_destinations_and_checks() {
        assert!(has(
            &with(
                r#"DestDir: "{app}\tip\{#AppVersion}"; Check: TipNeedsCopy; Flags: ignoreversion uninsrestartdelete"#,
                r#"DestDir: "{app}\tip\{#AppVersion}"; Check: TipNeedsCopy; Flags: ignoreversion regserver"#
            ),
            "regserver"
        ));
        assert!(has(
            &with(
                r#"Source: "{#StageDir}\ikari-engine-host.exe"; DestDir: "{app}"; Check: AppDirReady;"#,
                r#"Source: "{#StageDir}\ikari-engine-host.exe"; DestDir: "{app}";"#
            ),
            "Check"
        ));
        assert!(has(
            &with(
                r#"Source: "{#StageDir}\ikari-lm-host.exe"; DestDir: "{app}""#,
                r#"Source: "{#StageDir}\ikari-lm-host.exe"; DestDir: "{commonappdata}\ikari""#
            ),
            "置き先"
        ));
    }

    #[test]
    fn launches_go_through_one_trusted_place() {
        assert!(has(
            &with(
                "procedure FinishInstall;",
                "procedure Bad;\nvar C: Integer;\nbegin\n  Exec('x', '', '', 0, ewWaitUntilTerminated, C);\nend;\n\nprocedure FinishInstall;"
            ),
            "Exec("
        ));
        assert!(has(
            &with(
                "RunTrusted(SysTool('regsvr32.exe'), '/s",
                "RunTrusted('regsvr32.exe', '/s"
            ),
            "RunTrusted の相手"
        ));
    }

    #[test]
    fn a_launch_inside_step_ok_is_refused() {
        // 落ちる側: 第 1 引数の中で RunTrusted を呼ぶ形（Code が実行前の値になる）
        let bad = with(
            "  Result := RunTrusted(SysTool('regsvr32.exe'), '/s \"' + TipDll + '\"', Code);\n  Result := StepOk(Result, Code, 'DllRegisterServer');",
            "  Result := StepOk(RunTrusted(SysTool('regsvr32.exe'), '/s \"' + TipDll + '\"', Code), Code, 'DllRegisterServer');",
        );
        assert!(has(&bad, "StepOk の第 1 引数"), "{bad:#?}");
        // 通る側: 今の ikari.iss（代入してから StepOk）と、文字列の中のカンマや括弧
        assert!(!has(
            &with("StepOk(Started, Code,", "StepOk(Started, Code,"),
            "StepOk の第 1 引数"
        ));
        assert_eq!(
            first_argument("RegWrite(A, 'x,)'), 0, 'Run')"),
            "RegWrite(A, 'x,)')"
        );
    }

    #[test]
    fn orders_are_enforced() {
        assert!(has(
            &with(
                "  if not RegisterTip then\n    exit;\n  if not AddFirewallRules then\n    exit;\n",
                "  if not AddFirewallRules then\n    exit;\n  if not RegisterTip then\n    exit;\n"
            ),
            "FinishInstall の順"
        ));
        assert!(has(
            &with(
                "  UnregisterTip;\n  StopEngines(AppHelper);",
                "  StopEngines(AppHelper);\n  UnregisterTip;"
            ),
            "UninstallInOrder の順"
        ));
        assert!(has(
            &with(
                "  if not VerifyAppDir then\n  begin\n    RemoveNewUninstaller;\n    exit;\n  end;\n",
                ""
            ),
            "VerifyAppDir がありません"
        ));
    }

    #[test]
    fn install_steps_stop_at_the_first_failure() {
        // 規則に失敗しても自動起動まで進む形は落ちる
        let problems = with(
            "  if not AddFirewallRules then\n    exit;\n",
            "  AddFirewallRules;\n",
        );
        assert!(
            has(&problems, "AddFirewallRules が失敗したら"),
            "{problems:#?}"
        );
        assert!(has(
            &with("  if not RegisterTip then\n    exit;\n", "  RegisterTip;\n"),
            "RegisterTip が失敗したら"
        ));
    }

    #[test]
    fn stale_lm_libraries_are_removed_after_the_check_and_the_stop() {
        assert!(has(
            &with(
                "      AppDirTrusted := RemoveStaleLmLibraries;\n",
                "      AppDirTrusted := True;\n"
            ),
            "AppDirReady の順: RemoveStaleLmLibraries がありません"
        ));
        assert!(has(
            &with(
                "      StopEngines(WorkHelper);\n",
                "      RemoveStaleLmLibraries;\n      StopEngines(WorkHelper);\n"
            ),
            "AppDirReady の順"
        ));
    }

    #[test]
    fn preparation_waits_for_the_work_helper() {
        assert!(has(
            &with(" AfterInstall: WorkHelperPlaced;", ""),
            "AfterInstall: WorkHelperPlaced"
        ));
        assert!(has(
            &with("  if not WorkHelperReady then\n", "  if False then\n"),
            "道具を置く前"
        ));
    }

    #[test]
    fn the_uninstall_entry_follows_the_final_check() {
        assert!(has(
            &with(
                "CreateUninstallRegKey=UninstallEntryAllowed",
                "CreateUninstallRegKey=yes"
            ),
            "CreateUninstallRegKey"
        ));
        assert!(has(
            &with("CreateUninstallRegKey=UninstallEntryAllowed\n", ""),
            "CreateUninstallRegKey"
        ));
    }

    #[test]
    fn blocked_exes_follow_the_helper_and_rule_names_have_no_version() {
        assert!(has(
            &with("    1: Result := 'ikari-lm-host.exe';\n", ""),
            "BlockedExe の並び"
        ));
        // 設定画面（settings\ikari-settings.exe）を外すと、helper の止める相手と食い違う（#575）
        assert!(has(
            &with("    3: Result := 'settings\\ikari-settings.exe';\n", ""),
            "BlockedExe の並び"
        ));
        assert!(has(
            &with(
                "FirewallRulePrefix = 'ikari-ime: block outbound ';",
                "FirewallRulePrefix = 'ikari-ime {#AppVersion}: block outbound ';"
            ),
            "版を入れない"
        ));
        assert!(has(
            &with(
                "RegWriteStringValue(HKLM64, RunKey",
                "RegWriteStringValue(HKLM, RunKey"
            ),
            "HKLM64"
        ));
    }

    #[test]
    fn exit_codes_follow_the_helper() {
        assert!(has(
            &with("HelperUntrusted = 2;", "HelperUntrusted = 5;"),
            "HelperUntrusted"
        ));
    }

    #[test]
    fn helper_exes_are_read_in_order() {
        let stop = "pub const ENGINE_EXE: &str = \"a.exe\";\npub const BLOCKED_CHILD_EXES: &[&str] = &[\"b.exe\", r\"c\\d.exe\"];\npub const STOP_CHILD_EXES: &[&str] = &[\"b.exe\", r\"c\\d.exe\", \"data.exe\"];\n";
        assert_eq!(
            helper_blocked_exes(stop),
            vec!["a.exe", "b.exe", r"c\d.exe"]
        );
        assert_eq!(
            helper_stop_exes(stop),
            vec!["b.exe", r"c\d.exe", "data.exe"]
        );
        let folded = "pub const ENGINE_EXE: &str = \"a.exe\";\npub const BLOCKED_CHILD_EXES: &[&str] = &[\n    \"b.exe\",\n    r\"c\\d.exe\",\n];\n";
        assert_eq!(
            helper_blocked_exes(folded),
            vec!["a.exe", "b.exe", r"c\d.exe"]
        );
        assert!(helper_stop_exes(folded).is_empty());
    }

    #[test]
    fn a_stop_list_without_the_data_child_or_a_blocked_data_child_is_reported() {
        let real = repo_file(HELPER_STOP);
        let run = |stop: &str| {
            problems(
                &repo_file(ISS),
                &repo_file(HELPER_LIB),
                stop,
                &repo_file(SCAN_MACOS),
            )
        };
        let without_data = real.replacen("    \"ikari-data.exe\",\n", "", 1);
        assert_ne!(without_data, real);
        assert!(has(&run(&without_data), "STOP_CHILD_EXES"));
        let blocked_data = real.replacen(
            "pub const BLOCKED_CHILD_EXES: &[&str] = &[",
            "pub const BLOCKED_CHILD_EXES: &[&str] = &[\n    \"ikari-data.exe\",",
            1,
        );
        assert!(has(&run(&blocked_data), "BlockedExe の並び"));
    }

    #[test]
    fn edition_guard_and_offline_exclusion_of_data_child() {
        assert!(has(
            &with(
                "#define EditionSuffix \"-offline\"",
                "#define EditionSuffix \"\""
            ),
            "EditionSuffix"
        ));
        assert!(has(
            &with(
                "OutputBaseFilename=ikari-ime-{#AppVersion}-windows-x64-setup{#EditionSuffix}",
                "OutputBaseFilename=ikari-ime-{#AppVersion}-windows-x64-setup"
            ),
            "OutputBaseFilename"
        ));
        // ikari-data.exe を #if Edition == "normal" の外に出すと落ちる
        assert!(has(
            &with(
                "#if Edition == \"normal\"\n; 取得の子（ADR 0076。設定画面の核がインストール先の直下のこれを起動する。通常版だけ。ADR 0080 §7.3）。\n; 送信ブロックの規則はかけない（ADR 0076、0071 §3.1）\nSource: \"{#StageDir}\\ikari-data.exe\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion\n#endif",
                "Source: \"{#StageDir}\\ikari-data.exe\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion"
            ),
            "#if Edition == \"normal\""
        ));
        // 通常版から ikari-data.exe を消すと落ちる
        assert!(has(
            &with(
                "Source: \"{#StageDir}\\ikari-data.exe\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion\n",
                ""
            ),
            "通常版の `ikari-data.exe`"
        ));
        // #if Edition == "normal" の中に他の項目（同梱データなど）を入れると落ちる
        assert!(has(
            &with(
                "Source: \"{#StageDir}\\ikari-data.exe\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion\n#endif",
                "Source: \"{#StageDir}\\ikari-data.exe\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion\nSource: \"{#StageDir}\\LICENSE\"; DestDir: \"{app}\"; Check: AppDirReady; Flags: ignoreversion\n#endif"
            ),
            "`#if Edition == \"normal\"` の中に置くのは ikari-data.exe だけ"
        ));
        // [Code] に ikari-data.exe を書くと落ちる
        assert!(has(
            &with(
                "    3: Result := 'settings\\ikari-settings.exe';",
                "    3: Result := 'settings\\ikari-settings.exe';\n    4: Result := 'ikari-data.exe';"
            ),
            "`ikari-data.exe` は [Files] の通常版の項目のほかに書かない"
        ));
    }

    #[test]
    fn bundled_data_and_notices_follow_the_shared_table() {
        // 必須の辞書・モデル・NOTICE.txt・licenses\*・THIRD_PARTY_NOTICES.md を消すと落ちる
        for required in [
            r#"Source: "{#StageDir}\data\dict\grammar.bin"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\dict\system.dict"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\dict\vectors.bin"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\models\zenz-v3.2-small-Q5_K_M.gguf"; DestDir: "{app}\data\models"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\NOTICE.txt"; DestDir: "{app}\data"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\licenses\*"; DestDir: "{app}\data\licenses"; Check: AppDirReady; Flags: ignoreversion recursesubdirs"#,
            r#"Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion"#,
        ] {
            assert!(has(&with(required, ""), "がありません"), "{required}");
        }
        // 必須の項目に skipifsourcedoesntexist を付けると落ちる
        for required in [
            r#"Source: "{#StageDir}\data\dict\system.dict"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\models\zenz-v3.2-small-Q5_K_M.gguf"; DestDir: "{app}\data\models"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\data\NOTICE.txt"; DestDir: "{app}\data"; Check: AppDirReady; Flags: ignoreversion"#,
            r#"Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion"#,
        ] {
            let opt = format!("{required} skipifsourcedoesntexist");
            assert!(
                has(&with(required, &opt), "skipifsourcedoesntexist を付けない"),
                "{required}"
            );
        }
        // 任意の辞書から skipifsourcedoesntexist を外すと落ちる
        assert!(has(
            &with(
                r#"Source: "{#StageDir}\data\dict\emoji.tsv"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion skipifsourcedoesntexist"#,
                r#"Source: "{#StageDir}\data\dict\emoji.tsv"; DestDir: "{app}\data\dict"; Check: AppDirReady; Flags: ignoreversion"#
            ),
            "任意の辞書なので skipifsourcedoesntexist を付ける"
        ));
        // ワイルドカードや表にない項目を data\ に足すと落ちる
        assert!(has(
            &with(
                r#"Source: "{#StageDir}\data\NOTICE.txt"; DestDir: "{app}\data"; Check: AppDirReady; Flags: ignoreversion"#,
                "Source: \"{#StageDir}\\data\\NOTICE.txt\"; DestDir: \"{app}\\data\"; Check: AppDirReady; Flags: ignoreversion\nSource: \"{#StageDir}\\data\\models\\*.gguf\"; DestDir: \"{app}\\data\\models\"; Check: AppDirReady; Flags: ignoreversion"
            ),
            "表にない項目があります"
        ));
        // data\ の項目を版ごとのフォルダにすると落ちる（ADR 0080 §6）
        assert!(has(
            &with(
                r#"Source: "{#StageDir}\data\dict\system.dict"; DestDir: "{app}\data\dict""#,
                r#"Source: "{#StageDir}\data\dict\system.dict"; DestDir: "{app}\data\dict\{#AppVersion}""#
            ),
            r"DestDir は {app}\data\dict"
        ));
        // xtask/src/scan/macos.rs の表が変わったら食い違いで落ちる
        let changed_table = repo_file(SCAN_MACOS).replacen(
            "pub const DICT_REQUIRED: &[&str] = &[",
            "pub const DICT_REQUIRED: &[&str] = &[\"extra.bin\", ",
            1,
        );
        let found = problems(
            &repo_file(ISS),
            &repo_file(HELPER_LIB),
            &repo_file(HELPER_STOP),
            &changed_table,
        );
        assert!(has(&found, r"{#StageDir}\data\dict\extra.bin"));
    }

    #[test]
    fn stop_list_must_add_only_the_data_child_to_the_blocked_children() {
        let real = repo_file(HELPER_STOP);
        let blocked = helper_blocked_exes(&real);
        let stop = helper_stop_exes(&real);
        assert_eq!(&stop[..stop.len() - 1], &blocked[1..]);
        assert_eq!(stop.last().map(String::as_str), Some(STOP_DATA_EXE));
        // 取得の子が送信ブロックの側に入ると、一覧の照合が落ちる
        assert!(!blocked.iter().any(|b| b == STOP_DATA_EXE));
    }

    #[test]
    fn words_match_on_boundaries() {
        assert!(contains_word("regwritestringvalue(hka, 'x')", "hka"));
        assert!(!contains_word("hkaxx", "hka"));
        assert!(!contains_word("shkaa", "hka"));
    }
}
