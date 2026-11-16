//! 取得先の URL を厳しく読む（ADR 0076 §4.3）。URL の crate は足さない。
//!
//! 受けるもの: `https://` で始まり、全体が ASCII で、空白と制御文字がなく、2048 文字まで。ホストは ASCII の
//! 英数字、`-`、`.` だけ（国際化ドメインは `xn--` の形で入れてもらう。確認の画面にこのホストをそのまま出す）。
//! 利用者名とパスワード（`@`）、IP アドレスの角括弧（`[`）、フラグメント（`#`）は拒む。

use std::fmt;

/// URL の長さの上限（文字数。全体が ASCII なのでバイト数と同じ）
pub const MAX_URL_LEN: usize = 2048;
const SCHEME: &str = "https://";
/// DNS のホスト名の長さの上限（RFC 1035）
const MAX_HOST_LEN: usize = 253;
/// DNS のラベルの長さの上限（RFC 1035）
const MAX_LABEL_LEN: usize = 63;

/// 読んだ URL。`as_str` が要求に使う文字列、`host` が確認の画面に出すホスト
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl {
    text: String,
    host: String,
}

/// 読めない理由（行には出さない。`bad-url` の 1 語にまとまる）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    TooLong,
    NotHttps,
    NotAscii,
    Whitespace,
    Userinfo,
    Fragment,
    BadHost,
    BadPort,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLong => "URL が長すぎます",
            Self::NotHttps => "https:// で始まる URL だけ受けます",
            Self::NotAscii => "URL は ASCII だけで書きます（国際化ドメインは xn-- の形で）",
            Self::Whitespace => "URL に空白や制御文字があります",
            Self::Userinfo => "URL に利用者名やパスワードは書けません",
            Self::Fragment => "URL に # は書けません",
            Self::BadHost => "ホスト名が不正です",
            Self::BadPort => "ポート番号が不正です",
        })
    }
}

impl std::error::Error for UrlError {}

impl HttpsUrl {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        if input.len() > MAX_URL_LEN {
            return Err(UrlError::TooLong);
        }
        if !input.is_ascii() {
            return Err(UrlError::NotAscii);
        }
        if input.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(UrlError::Whitespace);
        }
        let rest = input.strip_prefix(SCHEME).ok_or(UrlError::NotHttps)?;
        if input.contains('#') {
            return Err(UrlError::Fragment);
        }
        let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        if authority.contains('@') {
            return Err(UrlError::Userinfo);
        }
        if authority.contains(['[', ']']) {
            return Err(UrlError::BadHost);
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if let Some(port) = port {
            let valid = !port.is_empty()
                && port.len() <= 5
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p));
            if !valid {
                return Err(UrlError::BadPort);
            }
        }
        check_host(host)?;
        Ok(Self {
            text: input.to_owned(),
            host: host.to_ascii_lowercase(),
        })
    }

    /// 要求に使う文字列（入力のまま）
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 小文字にした ASCII のホスト（ポートを除く）
    pub fn host(&self) -> &str {
        &self.host
    }
}

fn check_host(host: &str) -> Result<(), UrlError> {
    if host.is_empty() || host.len() > MAX_HOST_LEN {
        return Err(UrlError::BadHost);
    }
    for label in host.split('.') {
        let ok = !label.is_empty()
            && label.len() <= MAX_LABEL_LEN
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err(UrlError::BadHost);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_https_urls() {
        let u = HttpsUrl::parse(
            "https://HuggingFace.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/main/m.gguf?download=true",
        )
        .unwrap();
        assert_eq!(u.host(), "huggingface.co");
        assert!(u.as_str().starts_with("https://HuggingFace.co/"));
        assert_eq!(
            HttpsUrl::parse("https://example.com:8443/x")
                .unwrap()
                .host(),
            "example.com"
        );
        assert_eq!(
            HttpsUrl::parse("https://xn--n8jwe.example/")
                .unwrap()
                .host(),
            "xn--n8jwe.example"
        );
        assert!(HttpsUrl::parse("https://example.com").is_ok());
        assert!(HttpsUrl::parse("https://example.com?x=1").is_ok());
    }

    #[test]
    fn rejects_non_https() {
        for s in [
            "http://example.com/",
            "ftp://example.com/",
            "example.com",
            "",
            "HTTPS://example.com/",
            "//example.com/",
            "file:///etc/passwd",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::NotHttps), "{s:?}");
        }
    }

    #[test]
    fn rejects_userinfo_even_when_it_looks_like_a_host() {
        for s in [
            "https://user@example.com/",
            "https://user:pass@example.com/",
            "https://example.com@evil.example/",
            "https://@example.com/",
            "https://example.com:443@evil.example/",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::Userinfo), "{s:?}");
        }
        // パスの中の @ は、ホストを変えない
        assert_eq!(
            HttpsUrl::parse("https://example.com/a@b").unwrap().host(),
            "example.com"
        );
    }

    #[test]
    fn rejects_non_ascii_hosts_and_urls() {
        for s in [
            "https://例え.jp/",
            "https://exämple.com/",
            "https://example.com/日本語",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::NotAscii), "{s:?}");
        }
    }

    #[test]
    fn rejects_whitespace_and_control_characters() {
        for s in [
            "https://example.com/a b",
            "https://example.com/\n",
            "https://example.com/\t",
            "https://exa mple.com/",
            "https://example.com/\0",
            "https://example.com/\x7f",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::Whitespace), "{s:?}");
        }
    }

    #[test]
    fn rejects_bad_hosts_and_ports() {
        for s in [
            "https:///path",
            "https://./",
            "https://.example.com/",
            "https://example..com/",
            "https://-a.example/",
            "https://a-.example/",
            "https://[::1]/",
            "https://exa_mple.com/",
            "https://exa%6dple.com/",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::BadHost), "{s:?}");
        }
        for s in [
            "https://example.com:/",
            "https://example.com:0/",
            "https://example.com:65536/",
            "https://example.com:80a/",
            "https://example.com:123456/",
        ] {
            assert_eq!(HttpsUrl::parse(s), Err(UrlError::BadPort), "{s:?}");
        }
    }

    #[test]
    fn rejects_fragments_and_overlong_urls() {
        assert_eq!(
            HttpsUrl::parse("https://example.com/#x"),
            Err(UrlError::Fragment)
        );
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(HttpsUrl::parse(&long), Err(UrlError::TooLong));
        let just = format!(
            "https://example.com/{}",
            "a".repeat(MAX_URL_LEN - "https://example.com/".len())
        );
        assert!(HttpsUrl::parse(&just).is_ok());
    }
}
