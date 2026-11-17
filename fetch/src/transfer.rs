//! 通信（ADR 0076 §4.4）。**`ureq` を使うのはこのファイルだけ**（製品のソースの検査が見る。ADR 0076 §10）。
//!
//! - 設定は `build_config` の 1 か所で決める: TLS は OS のもの（`TlsProvider::NativeTls`）と OS の根の証明書
//!   （`RootCerts::PlatformVerifier`）、https だけ（始めの URL も転送先も）、転送の回数の上限（超えたら失敗）、
//!   接続・名前の解決・無通信の締め切り、全体の締め切りは置かない、UA は `ikari-data`（版なし）。
//!   Cookie、認証、圧縮は使わない（feature を入れていない）。証明書の確かめは外さない。
//! - プロキシは環境変数だけ（`Proxy::try_from_env()` を明示して渡す。ADR 0076 §3.4。Windows の OS の登録値は読まない）。
//! - 本文は `limit` で上限を守り、読んだバイト数を数えて進み具合を知らせる。
//! - 配る対象でない OS（Linux など）では、通信のコードを持たず、`FetchErrorCode::Tool` を返す。

use std::io::Write;
use std::time::Duration;

use ikari_resource::fetch_report::FetchErrorCode;

use crate::error::FetchError;
use crate::limits::{CONNECT_TIMEOUT, MAX_REDIRECTS, READ_TIMEOUT};
use crate::url::HttpsUrl;

/// 通信の締め切りと転送の上限
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetLimits {
    pub max_redirects: u32,
    /// 接続と名前の解決
    pub connect: Duration,
    /// 無通信（読むたびに数え直す）
    pub read: Duration,
}

impl NetLimits {
    /// 取得の既定（仮置き。`limits.rs`）
    pub const DOWNLOAD: NetLimits = NetLimits {
        max_redirects: MAX_REDIRECTS,
        connect: CONNECT_TIMEOUT,
        read: READ_TIMEOUT,
    };

    /// 版の確認（ADR 0076 §6）。転送は追わない（`Location` を読むだけ）。締め切りは取得と同じ仮置き
    pub const VERSION_CHECK: NetLimits = NetLimits {
        max_redirects: 0,
        connect: CONNECT_TIMEOUT,
        read: READ_TIMEOUT,
    };
}

/// 1 回の取得の頼み
#[derive(Debug, Clone)]
pub struct DownloadRequest<'a> {
    pub url: &'a HttpsUrl,
    /// 受けてよい大きさの上限（バイト）
    pub max_size: u64,
    /// 応答の `Content-Length` と、受けた大きさが一致すべき値（プリセットの表の大きさ）
    pub expect_size: Option<u64>,
}

/// `url` を取得して `out` に書く。`progress(受けたバイト, 全体)` を読むたびに呼ぶ。書いたバイト数を返す。
/// 配る対象でない OS では `Tool`。
pub fn download(
    request: &DownloadRequest<'_>,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<u64, FetchError> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let agent = net::agent(&NetLimits::DOWNLOAD);
        net::download_with(&agent, request, out, progress)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (request, out, progress);
        Err(FetchError(FetchErrorCode::Tool))
    }
}

/// `url` に転送を追わずに 1 回だけ要求し、応答の `Location` を返す（無ければ None）。本文は `body_limit` まで読んで
/// 捨てる（版の確認。ADR 0076 §6）。配る対象でない OS では `Tool`。
///
/// 転送を追わない（`NetLimits::VERSION_CHECK`。`max_redirects` が 0 なら、ureq は 3xx の応答をそのまま返し、
/// `TooManyRedirects` にしない）。`Location` は、3xx の応答のときだけ返す（200 などは None）。中身の検査は呼び出し側。
pub fn redirect_location(url: &HttpsUrl, body_limit: u64) -> Result<Option<String>, FetchError> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let agent = net::agent(&NetLimits::VERSION_CHECK);
        net::redirect_location_with(&agent, url, body_limit)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (url, body_limit);
        Err(FetchError(FetchErrorCode::Tool))
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod net {
    use std::io::{self, Read};

    use ureq::config::Config;
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    use ureq::{Agent, Error as UreqError, Proxy, Timeout};

    use super::*;
    use crate::limits::USER_AGENT;

    /// 1 回に読む大きさ
    const READ_CHUNK_BYTES: usize = 64 * 1024;

    /// 転送を追わずに 1 回だけ要求し、3xx の応答の `Location` を返す
    pub(super) fn redirect_location_with(
        agent: &Agent,
        url: &HttpsUrl,
        body_limit: u64,
    ) -> Result<Option<String>, FetchError> {
        let response = agent.get(url.as_str()).call().map_err(|e| map_error(&e))?;
        let location = if response.status().is_redirection() {
            response
                .headers()
                .get(ureq::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        } else {
            None
        };
        // 本文は読んで捨てる（上限まで。読めなくても、欲しかったのは Location だけなので失敗にしない）
        let mut body = response.into_body();
        let mut reader = body.with_config().limit(body_limit).reader();
        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        Ok(location)
    }

    /// 通信の設定。単体テストが、各項目を確かめる（ADR 0076 §10）
    pub(super) fn build_config(limits: &NetLimits, proxy: Option<Proxy>) -> Config {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        Config::builder()
            .tls_config(tls)
            .https_only(true)
            .max_redirects(limits.max_redirects)
            .max_redirects_will_error(true)
            .timeout_connect(Some(limits.connect))
            .timeout_resolve(Some(limits.connect))
            .timeout_per_read(Some(limits.read))
            .user_agent(USER_AGENT)
            .proxy(proxy)
            .build()
    }

    pub(super) fn agent(limits: &NetLimits) -> Agent {
        // 読む環境変数はプロキシだけ（ADR 0076 §3.4）
        build_config(limits, Proxy::try_from_env()).new_agent()
    }

    pub(super) fn download_with(
        agent: &Agent,
        request: &DownloadRequest<'_>,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, FetchError> {
        let response = agent
            .get(request.url.as_str())
            .call()
            .map_err(|e| map_error(&e))?;
        let total = response.body().content_length();
        if let Some(total) = total {
            if total > request.max_size {
                return Err(FetchErrorCode::TooLarge.into());
            }
            if request.expect_size.is_some_and(|expect| expect != total) {
                return Err(FetchErrorCode::Checksum.into());
            }
        }

        let mut body = response.into_body();
        let mut reader = body.with_config().limit(request.max_size).reader();
        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        let mut received = 0u64;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(map_error(&UreqError::from(e))),
            };
            received += n as u64;
            // 上限は `limit` が守る。ここは、念のための二重の確かめ
            if received > request.max_size {
                return Err(FetchErrorCode::TooLarge.into());
            }
            out.write_all(&buf[..n]).map_err(|e| {
                FetchError(if e.kind() == io::ErrorKind::StorageFull {
                    FetchErrorCode::DiskFull
                } else {
                    FetchErrorCode::Internal
                })
            })?;
            progress(received, total);
        }
        if request.expect_size.is_some_and(|expect| expect != received) {
            return Err(FetchErrorCode::Checksum.into());
        }
        Ok(received)
    }

    /// `ureq::Error` を理由の語にする（ADR 0076 §8.2 の表）。知らない種類は `internal`
    pub(super) fn map_error(e: &UreqError) -> FetchError {
        use FetchErrorCode as C;
        FetchError(match e {
            UreqError::StatusCode(_)
            | UreqError::TooManyRedirects
            | UreqError::RedirectFailed
            | UreqError::RequireHttpsOnly(_) => C::HttpStatus,
            UreqError::Timeout(Timeout::Connect | Timeout::Resolve) => C::Network,
            UreqError::Timeout(_) | UreqError::BodyStalled => C::Stalled,
            UreqError::BodyExceedsLimit(_) => C::TooLarge,
            UreqError::NativeTls(_) | UreqError::Tls(_) | UreqError::TlsRequired => C::Tls,
            UreqError::Io(_)
            | UreqError::HostNotFound
            | UreqError::ConnectionFailed
            | UreqError::ConnectProxyFailed(_)
            | UreqError::InvalidProxyUrl => C::Network,
            UreqError::BadUri(_) => C::BadUrl,
            _ => C::Internal,
        })
    }

    #[cfg(test)]
    mod tests {
        use std::net::{TcpListener, TcpStream};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;

        use ureq::tls::TlsProvider;
        use ureq::unversioned::resolver::DefaultResolver;
        use ureq::unversioned::transport::{
            Buffers, ConnectionDetails, Connector, NextTimeout, TcpConnector, Transport,
        };

        use super::*;

        const TEST_LIMITS: NetLimits = NetLimits {
            max_redirects: 3,
            connect: Duration::from_secs(5),
            read: Duration::from_millis(300),
        };

        // ---- 設定（ADR 0076 §10「通信の設定」）

        #[test]
        fn config_matches_the_contract() {
            let c = build_config(&NetLimits::DOWNLOAD, None);
            assert!(c.https_only());
            assert_eq!(c.max_redirects(), MAX_REDIRECTS);
            assert!(c.max_redirects_will_error());
            assert_eq!(c.timeouts().connect, Some(CONNECT_TIMEOUT));
            assert_eq!(c.timeouts().resolve, Some(CONNECT_TIMEOUT));
            assert_eq!(c.timeout_per_read(), Some(READ_TIMEOUT));
            // 全体の締め切りは置かない（大きいモデルが遅い回線で落ちないため）
            assert_eq!(c.timeouts().global, None);
            assert_eq!(c.timeouts().per_call, None);
            assert!(c.http_status_as_error());
            assert_eq!(c.tls_config().provider(), TlsProvider::NativeTls);
            assert!(matches!(
                c.tls_config().root_certs(),
                RootCerts::PlatformVerifier
            ));
            assert!(!c.tls_config().disable_verification());
            assert!(c.proxy().is_none());
            assert_eq!(
                format!("{:?}", c.user_agent()),
                format!("{:?}", ureq::config::AutoHeaderValue::from(USER_AGENT))
            );
        }

        #[test]
        fn version_check_config_does_not_follow_redirects_and_stays_strict() {
            let c = build_config(&NetLimits::VERSION_CHECK, None);
            assert_eq!(c.max_redirects(), 0);
            assert!(c.https_only());
            assert!(c.http_status_as_error());
            assert!(!c.tls_config().disable_verification());
        }

        fn locate(url: &str) -> Result<Option<String>, FetchError> {
            let url = HttpsUrl::parse(url).unwrap();
            let agent = Agent::with_parts(
                build_config(&NetLimits::VERSION_CHECK, None),
                PlainAsSecure::default(),
                DefaultResolver::default(),
            );
            redirect_location_with(&agent, &url, 1024)
        }

        #[test]
        fn a_302_is_read_without_following_it() {
            let (port, hits) = serve(|_, _| redirect("https://example.com/releases/tag/v0.3.1"));
            let got = locate(&format!("https://127.0.0.1:{port}/latest")).unwrap();
            assert_eq!(
                got.as_deref(),
                Some("https://example.com/releases/tag/v0.3.1")
            );
            assert_eq!(hits.load(Ordering::SeqCst), 1, "転送先には行かない");
        }

        #[test]
        fn a_reply_without_a_redirect_has_no_location() {
            let (port, _) = serve(|_, _| ok(b"hello"));
            assert_eq!(
                locate(&format!("https://127.0.0.1:{port}/latest")).unwrap(),
                None
            );
        }

        #[test]
        fn a_redirect_without_location_has_no_location() {
            let (port, _) = serve(|_, _| {
                b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            });
            assert_eq!(
                locate(&format!("https://127.0.0.1:{port}/latest")).unwrap(),
                None
            );
        }

        #[test]
        fn version_check_errors_keep_their_words() {
            let (port, _) = serve(|_, _| {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            });
            assert_eq!(
                locate(&format!("https://127.0.0.1:{port}/latest"))
                    .unwrap_err()
                    .code(),
                FetchErrorCode::HttpStatus
            );
        }

        #[test]
        fn proxy_is_passed_through_only_when_given() {
            let proxy = Proxy::new("http://proxy.example:3128").unwrap();
            let c = build_config(&NetLimits::DOWNLOAD, Some(proxy));
            assert!(c.proxy().is_some());
        }

        // ---- 小さな HTTP サーバ（平文。TLS のない接続を、https の URL に見せて使う）

        /// 接続のたびに `handler(要求のパス)` が返した応答の生のバイト列を返す
        fn serve(
            handler: impl Fn(&str, u16) -> Vec<u8> + Send + Sync + 'static,
        ) -> (u16, Arc<AtomicUsize>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicUsize::new(0));
            let counter = hits.clone();
            let handler = Arc::new(handler);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { return };
                    counter.fetch_add(1, Ordering::SeqCst);
                    let handler = handler.clone();
                    thread::spawn(move || handle(stream, port, &*handler));
                }
            });
            (port, hits)
        }

        fn handle(mut stream: TcpStream, port: u16, handler: &dyn Fn(&str, u16) -> Vec<u8>) {
            let mut buf = [0u8; 2048];
            let mut head = Vec::new();
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let text = String::from_utf8_lossy(&head).into_owned();
            let path = text.split(' ').nth(1).unwrap_or("/").to_owned();
            let response = handler(&path, port);
            let _ = stream.write_all(&response);
            let _ = stream.flush();
            // 本文を途中で止めたいテストのために、閉じずに少し待つ
            thread::sleep(Duration::from_millis(1500));
        }

        fn ok(body: &[u8]) -> Vec<u8> {
            let mut v = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            v.extend_from_slice(body);
            v
        }

        fn redirect(location: &str) -> Vec<u8> {
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .into_bytes()
        }

        /// 平文の TCP を「TLS 済み」と偽って返す（https の URL を、手元の平文のサーバに向けるため。
        /// TLS そのものは、OS のもので、ここでは確かめない）
        #[derive(Debug)]
        struct SecureLie(Box<dyn Transport>);

        impl Transport for SecureLie {
            fn buffers(&mut self) -> &mut dyn Buffers {
                self.0.buffers()
            }
            fn transmit_output(
                &mut self,
                amount: usize,
                timeout: NextTimeout,
            ) -> Result<(), UreqError> {
                self.0.transmit_output(amount, timeout)
            }
            fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, UreqError> {
                self.0.await_input(timeout)
            }
            fn is_open(&mut self) -> bool {
                self.0.is_open()
            }
            fn is_tls(&self) -> bool {
                true
            }
        }

        #[derive(Debug, Default)]
        struct PlainAsSecure(TcpConnector);

        impl Connector for PlainAsSecure {
            type Out = SecureLie;
            fn connect(
                &self,
                details: &ConnectionDetails,
                chained: Option<()>,
            ) -> Result<Option<SecureLie>, UreqError> {
                let inner = self.0.connect(details, chained)?;
                Ok(inner.map(|t| SecureLie(t.boxed())))
            }
        }

        fn test_agent() -> Agent {
            Agent::with_parts(
                build_config(&TEST_LIMITS, None),
                PlainAsSecure::default(),
                DefaultResolver::default(),
            )
        }

        fn run(
            url: &str,
            max_size: u64,
            expect_size: Option<u64>,
        ) -> (Result<u64, FetchError>, Vec<u8>) {
            let url = HttpsUrl::parse(url).unwrap();
            let request = DownloadRequest {
                url: &url,
                max_size,
                expect_size,
            };
            let mut out = Vec::new();
            let result = download_with(&test_agent(), &request, &mut out, &mut |_, _| {});
            (result, out)
        }

        #[test]
        fn downloads_the_body_and_reports_progress() {
            let (port, _) = serve(|_, _| ok(b"hello model"));
            let url = HttpsUrl::parse(&format!("https://127.0.0.1:{port}/m.gguf")).unwrap();
            let request = DownloadRequest {
                url: &url,
                max_size: 100,
                expect_size: Some(11),
            };
            let mut out = Vec::new();
            let mut seen = Vec::new();
            let n = download_with(&test_agent(), &request, &mut out, &mut |r, t| {
                seen.push((r, t))
            })
            .unwrap();
            assert_eq!(n, 11);
            assert_eq!(out, b"hello model");
            assert_eq!(seen.last(), Some(&(11, Some(11))));
        }

        #[test]
        fn plain_http_is_refused_by_the_agent_before_connecting() {
            // `HttpsUrl` が先に断るが、設定の `https_only` も、直に渡された http:// を接続の前に断る
            let (port, hits) = serve(|_, _| ok(b"x"));
            let err = test_agent()
                .get(format!("http://127.0.0.1:{port}/"))
                .call()
                .unwrap_err();
            assert!(matches!(err, UreqError::RequireHttpsOnly(_)), "{err}");
            assert_eq!(map_error(&err).code(), FetchErrorCode::HttpStatus);
            assert_eq!(hits.load(Ordering::SeqCst), 0, "接続していない");
        }

        #[test]
        fn follows_redirects_within_the_limit() {
            let (port, _) = serve(|path, port| match path {
                "/a" => redirect(&format!("https://127.0.0.1:{port}/b")),
                "/b" => redirect(&format!("https://127.0.0.1:{port}/c")),
                _ => ok(b"done"),
            });
            let (result, out) = run(&format!("https://127.0.0.1:{port}/a"), 100, None);
            assert_eq!(result.unwrap(), 4);
            assert_eq!(out, b"done");
        }

        #[test]
        fn too_many_redirects_fail_as_http_status() {
            // 上限は 3 回。4 回目の転送で失敗する
            let (port, _) = serve(|_, port| redirect(&format!("https://127.0.0.1:{port}/loop")));
            let (result, out) = run(&format!("https://127.0.0.1:{port}/loop"), 100, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::HttpStatus);
            assert!(out.is_empty());
        }

        #[test]
        fn redirect_to_plain_http_is_refused() {
            let (port, hits) = serve(|_, port| redirect(&format!("http://127.0.0.1:{port}/next")));
            let (result, _) = run(&format!("https://127.0.0.1:{port}/a"), 100, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::HttpStatus);
            assert_eq!(hits.load(Ordering::SeqCst), 1, "転送先には接続していない");
        }

        #[test]
        fn error_statuses_fail() {
            let (port, _) = serve(|_, _| {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            });
            let (result, _) = run(&format!("https://127.0.0.1:{port}/x"), 100, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::HttpStatus);
        }

        #[test]
        fn content_length_over_the_limit_fails_before_reading() {
            let (port, _) = serve(|_, _| ok(&[7u8; 50]));
            let (result, out) = run(&format!("https://127.0.0.1:{port}/x"), 10, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::TooLarge);
            assert!(out.is_empty());
        }

        #[test]
        fn chunked_body_over_the_limit_fails_while_reading() {
            // Content-Length がない応答でも、上限で止まる
            let (port, _) = serve(|_, _| {
                let mut v =
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        .to_vec();
                for _ in 0..4 {
                    v.extend_from_slice(b"8\r\n12345678\r\n");
                }
                v.extend_from_slice(b"0\r\n\r\n");
                v
            });
            let (result, out) = run(&format!("https://127.0.0.1:{port}/x"), 20, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::TooLarge);
            assert!(out.len() <= 20);
        }

        #[test]
        fn content_length_differing_from_the_table_is_a_checksum_error() {
            let (port, _) = serve(|_, _| ok(b"abc"));
            let (result, out) = run(&format!("https://127.0.0.1:{port}/x"), 100, Some(5));
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::Checksum);
            assert!(out.is_empty());
        }

        #[test]
        fn a_stalled_body_fails_as_stalled() {
            let (port, _) = serve(|_, _| {
                // 10 バイトと言って 3 バイトだけ送り、止まる
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_vec()
            });
            let (result, _) = run(&format!("https://127.0.0.1:{port}/x"), 100, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::Stalled);
        }

        #[test]
        fn unreachable_hosts_fail_as_network() {
            // 閉じたポート（bind して捨てる）
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let (result, _) = run(&format!("https://127.0.0.1:{port}/x"), 100, None);
            assert_eq!(result.unwrap_err().code(), FetchErrorCode::Network);
        }

        #[test]
        fn ureq_errors_map_to_the_documented_words() {
            use FetchErrorCode as C;
            let cases: Vec<(UreqError, C)> = vec![
                (UreqError::StatusCode(500), C::HttpStatus),
                (UreqError::TooManyRedirects, C::HttpStatus),
                (UreqError::RequireHttpsOnly("x".into()), C::HttpStatus),
                (UreqError::Timeout(Timeout::Connect), C::Network),
                (UreqError::Timeout(Timeout::Resolve), C::Network),
                (UreqError::Timeout(Timeout::PerRead), C::Stalled),
                (UreqError::BodyExceedsLimit(1), C::TooLarge),
                (UreqError::TlsRequired, C::Tls),
                (UreqError::HostNotFound, C::Network),
                (UreqError::ConnectionFailed, C::Network),
                (UreqError::BadUri("x".into()), C::BadUrl),
                (UreqError::InvalidProxyUrl, C::Network),
            ];
            for (e, want) in cases {
                assert_eq!(map_error(&e).code(), want, "{e}");
            }
        }
    }
}
