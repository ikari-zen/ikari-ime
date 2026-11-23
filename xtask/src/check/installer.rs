//! Windows のインストーラ（`windows/installer/ikari.iss`）の静的な検査（ADR 0067 §3・§4、0068 §6.2、0070 §4、0073 §4）。
//! Inno Setup は Mac で動かせないので、守ることを文字列と設定で確かめる。実際の動き（ACL、登録、規則、自動起動、
//! 止める側、エラー報告の除外）は、実機の手順書（`docs/windows-real-device-check.md` の項目 47〜）で確かめる。
//!
//! 見ること:
//!   - Inno Setup の下限の版（6.6.0）を、コンパイルのときに要求していること
//!   - `[Setup]` の安全に関わる値（管理者、固定のインストール先、使用中のアプリを閉じない、ログを出さない、64 ビット）
//!   - 利用者ごとの場所・利用者が書ける一時の場所・シェル・PowerShell に触れていないこと
//!   - 外のプログラムの起動が 1 か所（`RunTrusted`）で、相手が system32 の道具か helper だけであること
//!   - DLL の登録に `regserver` の印、置き換えに `restartreplace` の印を使っていないこと
//!   - `[Files]` の置き先が、インストール先か作業用のフォルダだけで、インストール先の項目が検査（Check）を通ること
//!   - インストールと削除の手順の順番（ADR 0067 §3.2）。インストールの手順は、失敗したら後を打ち切ること
//!   - 置く前の準備（検査 → エンジンの停止 → 前の版の LM の部品の掃除）の順番と、準備が作業用のフォルダに
//!     道具を置いたあとに始まること（Inno は、ファイルを置く前の大きさの見積もりでも Check を呼ぶ）
//!   - 削除の情報の登録（`CreateUninstallRegKey`）が、置いたあとの検査に通ったときだけであること
//!   - ファイアウォールとエラー報告の対象の exe が、helper の止める相手と同じ並びで、規則の名前に版が入らないこと
//!   - 自動起動が HKLM の Run であること
//!   - helper の終了コードが、helper の定義と同じ値であること

use std::path::Path;

use crate::data::run::{Failure, log};

const ISS: &str = "windows/installer/ikari.iss";
const HELPER_LIB: &str = "windows/installer/helper/src/lib.rs";
const HELPER_STOP: &str = "windows/installer/helper/src/stop.rs";

/// Inno Setup の下限（一時の場所の名前を CSPRNG で作る、SYSTEM のときの SystemTemp。2026-11 の調査）
const MIN_INNO_GUARD: &str = "#if Ver < EncodeVer(6, 6, 0)";

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
    log("Windows のインストーラの静的な検査（windows/installer/ikari.iss。ADR 0067 §3・§4）");
    let iss = read(root, ISS)?;
    let lib = read(root, HELPER_LIB)?;
    let stop = read(root, HELPER_STOP)?;
    let problems = problems(&iss, &lib, &stop);
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

fn problems(iss: &str, helper_lib: &str, helper_stop: &str) -> Vec<String> {
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
    for line in lines.iter().filter(|l| l.section == "files") {
        let params = parameters(line.text);
        let flags = params
            .iter()
            .find(|(k, _)| k == "flags")
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default();
        for (flag, why) in FORBIDDEN_FILE_FLAGS {
            if flags.split_whitespace().any(|f| f == *flag) {
                found.push(format!(
                    "[Files] の印 {flag} を使わない（{why}）: {}",
                    line.text
                ));
            }
        }
        let dest = params
            .iter()
            .find(|(k, _)| k == "destdir")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
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
        if dest.starts_with("{app}") {
            let check = params
                .iter()
                .find(|(k, _)| k == "check")
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            if !APP_CHECKS.contains(&check) {
                found.push(format!(
                    "[Files] のインストール先の項目は Check: {} を通す: {}",
                    APP_CHECKS.join(" か "),
                    line.text
                ));
            }
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
        let run = |stop: &str| problems(&repo_file(ISS), &repo_file(HELPER_LIB), stop);
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
