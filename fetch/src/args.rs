//! コマンドの引数（ADR 0076 §4.1）。人が打っても、設定画面の核が起動しても同じ形。
//!
//! ```text
//! ikari-data model fetch --preset <id>
//! ikari-data model fetch --url <https の URL> --format <形式> [--sha256 <64 桁の 16 進>] [--name <表示名>]
//! ikari-data version-check
//! ikari-data cleanup
//! ikari-data transfer --url <URL> --out <ファイル> --max-size <バイト> [--expect-size <バイト>]   （内部。Windows の子）
//! ```
//!
//! 知らない引数、重なった引数、足りない引数は `Usage`（何もしない）。データの場所を変える引数は持たない（ADR 0067）。

use std::path::PathBuf;

use ikari_resource::sha256::is_sha256_hex;

/// 表示名の長さの上限（文字数）
const MAX_NAME_CHARS: usize = 64;
/// 形式の語の長さの上限
const MAX_FORMAT_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    ModelFetch(ModelSource),
    VersionCheck,
    Cleanup,
    /// 通信する子の仕事（`ikari-data` が自分の子にだけ使う。使い方の表示には出さない）
    Transfer(TransferArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSource {
    Preset {
        id: String,
    },
    Url {
        url: String,
        format: String,
        /// 小文字の 16 進 64 文字
        sha256: Option<String>,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferArgs {
    pub url: String,
    pub out: PathBuf,
    pub max_size: u64,
    pub expect_size: Option<u64>,
}

/// 引数が読めない
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage;

/// `args` は、実行ファイルの名前を除いたもの
pub fn parse(args: &[String]) -> Result<Command, Usage> {
    let (head, rest) = args.split_first().ok_or(Usage)?;
    match head.as_str() {
        "model" => {
            let (sub, rest) = rest.split_first().ok_or(Usage)?;
            if sub != "fetch" {
                return Err(Usage);
            }
            parse_model_fetch(rest)
        }
        "version-check" if rest.is_empty() => Ok(Command::VersionCheck),
        "cleanup" if rest.is_empty() => Ok(Command::Cleanup),
        "transfer" => parse_transfer(rest),
        _ => Err(Usage),
    }
}

/// `--key value` の並びを、重なりなしで読む。`allowed` にない key は `Usage`
fn pairs<'a>(args: &'a [String], allowed: &[&str]) -> Result<Vec<(&'a str, &'a str)>, Usage> {
    if !args.len().is_multiple_of(2) {
        return Err(Usage);
    }
    let mut out: Vec<(&str, &str)> = Vec::new();
    for pair in args.as_chunks::<2>().0 {
        let key = pair[0].strip_prefix("--").ok_or(Usage)?;
        if !allowed.contains(&key) || out.iter().any(|(k, _)| *k == key) {
            return Err(Usage);
        }
        out.push((key, pair[1].as_str()));
    }
    Ok(out)
}

fn get<'a>(pairs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn parse_model_fetch(args: &[String]) -> Result<Command, Usage> {
    let p = pairs(args, &["preset", "url", "format", "sha256", "name"])?;
    let preset = get(&p, "preset");
    let url = get(&p, "url");
    match (preset, url) {
        (Some(id), None) => {
            // プリセットには、ほかの引数を付けない（表が決める）
            if p.len() != 1 {
                return Err(Usage);
            }
            Ok(Command::ModelFetch(ModelSource::Preset {
                id: id.to_owned(),
            }))
        }
        (None, Some(url)) => {
            let format = get(&p, "format").ok_or(Usage)?;
            if !is_format_word(format) {
                return Err(Usage);
            }
            let sha256 = get(&p, "sha256")
                .map(|s| {
                    let lower = s.to_ascii_lowercase();
                    is_sha256_hex(&lower).then_some(lower).ok_or(Usage)
                })
                .transpose()?;
            let name = get(&p, "name")
                .map(|n| is_display_name(n).then(|| n.to_owned()).ok_or(Usage))
                .transpose()?;
            Ok(Command::ModelFetch(ModelSource::Url {
                url: url.to_owned(),
                format: format.to_owned(),
                sha256,
                name,
            }))
        }
        _ => Err(Usage),
    }
}

fn parse_transfer(args: &[String]) -> Result<Command, Usage> {
    let p = pairs(args, &["url", "out", "max-size", "expect-size"])?;
    let number = |key: &str| -> Result<Option<u64>, Usage> {
        get(&p, key)
            .map(|v| {
                (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| v.parse().ok())
                    .flatten()
                    .ok_or(Usage)
            })
            .transpose()
    };
    Ok(Command::Transfer(TransferArgs {
        url: get(&p, "url").ok_or(Usage)?.to_owned(),
        out: PathBuf::from(get(&p, "out").ok_or(Usage)?),
        max_size: number("max-size")?.ok_or(Usage)?,
        expect_size: number("expect-size")?,
    }))
}

/// 形式の語（`PromptFormat`）。中身の判断はエンジン。ここは manifest に書ける安全な形かだけ
fn is_format_word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_FORMAT_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 表示名。制御文字を含まず、64 文字まで（manifest に文字として残すだけで、ファイル名には使わない）
fn is_display_name(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= MAX_NAME_CHARS && !s.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(line: &str) -> Result<Command, Usage> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args)
    }

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn parses_every_documented_form() {
        assert_eq!(
            parse_str("model fetch --preset zenz-small"),
            Ok(Command::ModelFetch(ModelSource::Preset {
                id: "zenz-small".into()
            }))
        );
        assert_eq!(parse_str("version-check"), Ok(Command::VersionCheck));
        assert_eq!(parse_str("cleanup"), Ok(Command::Cleanup));
        assert_eq!(
            parse_str(&format!(
                "model fetch --url https://example.com/m.gguf --format zenz --sha256 {} --name my-model",
                SHA.to_ascii_uppercase()
            )),
            Ok(Command::ModelFetch(ModelSource::Url {
                url: "https://example.com/m.gguf".into(),
                format: "zenz".into(),
                sha256: Some(SHA.into()),
                name: Some("my-model".into()),
            }))
        );
        assert_eq!(
            parse_str("transfer --url https://e.com/x --out /tmp/a --max-size 10 --expect-size 7"),
            Ok(Command::Transfer(TransferArgs {
                url: "https://e.com/x".into(),
                out: PathBuf::from("/tmp/a"),
                max_size: 10,
                expect_size: Some(7),
            }))
        );
    }

    #[test]
    fn unknown_missing_and_repeated_arguments_are_usage() {
        for line in [
            "",
            "model",
            "model fetch",
            "model fetch --preset",
            "model import --file x",
            "model fetch --preset a --preset b",
            "model fetch --preset a --url https://e.com/",
            "model fetch --preset a --format zenz",
            "model fetch --url https://e.com/",
            "model fetch --url https://e.com/ --format zenz --data-dir /x",
            "model fetch --url https://e.com/ --format bad/format",
            "model fetch --url https://e.com/ --format zenz --sha256 abc",
            "model fetch --url https://e.com/ --format zenz --url https://e.com/b",
            "model fetch preset zenz-small",
            "version-check --now",
            "cleanup extra",
            "transfer --url https://e.com/ --out /x",
            "transfer --url https://e.com/ --out /x --max-size -1",
            "transfer --url https://e.com/ --out /x --max-size 1 --expect-size x",
            "unknown",
        ] {
            assert_eq!(parse_str(line), Err(Usage), "{line:?}");
        }
    }

    #[test]
    fn display_names_refuse_control_characters_and_long_text() {
        let long = "a".repeat(MAX_NAME_CHARS + 1);
        let with_newline = vec![
            "model",
            "fetch",
            "--url",
            "https://e.com/",
            "--format",
            "zenz",
            "--name",
            "a\nb",
        ];
        let args: Vec<String> = with_newline.into_iter().map(str::to_owned).collect();
        assert_eq!(parse(&args), Err(Usage));
        assert_eq!(
            parse_str(&format!(
                "model fetch --url https://e.com/ --format zenz --name {long}"
            )),
            Err(Usage)
        );
    }
}
