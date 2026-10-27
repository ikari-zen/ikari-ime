//! 要求の入口: バイト列の検査、種類ごとの振り分け、ログ、返事のエンコード。

use super::*;

/// 要求 1 件の結果（返事のバイト列と、ログに書く固定の語だけ）
pub(crate) struct Processed {
    pub(crate) reply: Vec<u8>,
    pub(crate) kind: &'static str,
    pub(crate) error: Option<&'static str>,
    /// 文節の数、区切り直しの向き。ユーザーの文字列は入れない
    pub(crate) note: Option<String>,
}

impl Processed {
    pub(crate) fn failed(reply: Vec<u8>, kind: &'static str, code: ErrorCode) -> Processed {
        Processed {
            reply,
            kind,
            error: Some(code.as_str_name()),
            note: None,
        }
    }
}

pub(crate) fn request_kind(body: &envelope::Body) -> &'static str {
    match body {
        envelope::Body::Hello(_) => "hello",
        envelope::Body::BeginSession(_) => "begin_session",
        envelope::Body::EndSession(_) => "end_session",
        envelope::Body::Prefetch(_) => "prefetch",
        envelope::Body::Convert(_) => "convert",
        envelope::Body::Page(_) => "page",
        envelope::Body::Commit(_) => "commit",
        envelope::Body::CommitText(_) => "commit_text",
        envelope::Body::Cancel(_) => "cancel",
        envelope::Body::Shutdown(_) => "shutdown",
        envelope::Body::SegmentPage(_) => "segment_page",
        envelope::Body::ResizeSegment(_) => "resize_segment",
        envelope::Body::CommitSegments(_) => "commit_segments",
        envelope::Body::Reconvert(_) => "reconvert",
        envelope::Body::ListUserDict(_) => "list_user_dict",
        envelope::Body::AddUserDict(_) => "add_user_dict",
        envelope::Body::RemoveUserDict(_) => "remove_user_dict",
        envelope::Body::LearningStatus(_) => "learning_status",
        envelope::Body::ResetLearning(_) => "reset_learning",
        envelope::Body::ExportData(_) => "export_data",
        envelope::Body::ImportData(_) => "import_data",
        envelope::Body::GetProfile(_) => "get_profile",
        envelope::Body::SetProfile(_) => "set_profile",
        envelope::Body::EmojiTable(_) => "emoji_table",
        envelope::Body::PostalLookup(_) => "postal_lookup",
        envelope::Body::Diag(_) => diag::KIND_DIAG,
        _ => "other",
    }
}

/// 要求が持つ `session_id`（ログの `session=` 用。番号だけで文字ではない）
fn request_session(body: &envelope::Body) -> Option<u64> {
    match body {
        envelope::Body::Prefetch(r) => Some(r.session_id),
        envelope::Body::Convert(r) => Some(r.session_id),
        envelope::Body::CommitText(r) => Some(r.session_id),
        envelope::Body::Reconvert(r) => Some(r.session_id),
        envelope::Body::EndSession(r) => Some(r.session_id),
        _ => None,
    }
}

/// 要求が持つ `conversion_id`（ログの `conv=` 用。番号だけで文字ではない）
fn request_conversion(body: &envelope::Body) -> Option<u64> {
    match body {
        envelope::Body::Page(r) => Some(r.conversion_id),
        envelope::Body::Commit(r) => Some(r.conversion_id),
        envelope::Body::Cancel(r) => Some(r.conversion_id),
        envelope::Body::SegmentPage(r) => Some(r.conversion_id),
        envelope::Body::ResizeSegment(r) => Some(r.conversion_id),
        envelope::Body::CommitSegments(r) => Some(r.conversion_id),
        _ => None,
    }
}

/// 返事が新しく作った `conversion_id`（`convert` と `reconvert`）
fn reply_conversion(reply: &envelope::Body) -> Option<u64> {
    match reply {
        envelope::Body::ConvertReply(r) => Some(r.conversion_id),
        envelope::Body::SegmentLayout(r) => Some(r.conversion_id),
        _ => None,
    }
}

/// 変換の行を、同じセッションの `sh_episode` と、同じ変換の `commit` などに結ぶ番号の注記。
/// `session=` は変換を作る要求（`convert`、`reconvert`）にだけ付け、`conv=` は変換に触れる行すべてに付ける
/// （どれの `lm_applied` のまま確定したかを調べるため。ADR 0034 §4.3）
fn ids_note(session: Option<u64>, conversion: Option<u64>) -> Option<String> {
    match (session, conversion) {
        (Some(s), Some(c)) => Some(format!("session={s} conv={c}")),
        (Some(s), None) => Some(session_note(s)),
        (None, Some(c)) => Some(format!("conv={c}")),
        (None, None) => None,
    }
}

/// `session=` の注記
fn session_note(id: u64) -> String {
    format!("session={id}")
}

/// ログの注記を空白でつなぐ
fn join_notes(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    }
}

impl Inner {
    /// 要求 1 件を処理して、(返事, 要求の種類, エラーの種類) を返す。
    pub(crate) fn process(&self, req: &[u8]) -> Processed {
        if req.len() > MAX_ADMIN_REQUEST_BYTES {
            let code = ErrorCode::BadRequest;
            return Processed::failed(
                error_envelope(0, code, "request too large"),
                "invalid",
                code,
            );
        }
        let Ok(env) = Envelope::decode(req) else {
            let code = ErrorCode::BadRequest;
            return Processed::failed(error_envelope(0, code, "invalid message"), "invalid", code);
        };
        let request_id = env.request_id;
        let Some(body) = env.body else {
            let code = ErrorCode::BadRequest;
            return Processed::failed(
                error_envelope(request_id, code, "empty message"),
                "invalid",
                code,
            );
        };
        let kind = request_kind(&body);
        let resize = segments::resize_direction(&body);
        // 上限は 2 段。大きな要求を許すのは ImportData だけ
        if req.len() > MAX_REQUEST_BYTES && !matches!(body, envelope::Body::ImportData(_)) {
            let code = ErrorCode::BadRequest;
            return Processed::failed(
                error_envelope(request_id, code, "request too large"),
                kind,
                code,
            );
        }
        // 版が違っても Shutdown だけは受ける（殻が古いエンジンを止めるため）
        let is_shutdown = matches!(body, envelope::Body::Shutdown(_));
        if env.schema_version != SCHEMA_VERSION && !is_shutdown {
            let code = ErrorCode::SchemaMismatch;
            return Processed::failed(
                error_envelope(request_id, code, "schema version mismatch"),
                kind,
                code,
            );
        }
        let session = request_session(&body);
        let creates_conversion = matches!(
            body,
            envelope::Body::Convert(_) | envelope::Body::Reconvert(_)
        );
        let request_conv = request_conversion(&body);
        let result = catch_unwind(AssertUnwindSafe(|| self.dispatch(body)))
            .unwrap_or(Err(Fail::new(ErrorCode::Internal, "internal error")));
        match result {
            Ok((reply, trace)) => {
                // 管理の失敗は Ok の返事の中の DataIssue で返る。ログには種類の名前だけを書く
                let issue = admin::issue_label(&reply);
                // 文節の数と区切り直しの向きだけを添える（読みと表記は書かない。NFR-19）
                let ids = ids_note(
                    session.filter(|_| creates_conversion),
                    request_conv.or_else(|| reply_conversion(&reply)),
                );
                let note = join_notes(ids, join_notes(segments::log_note(&reply, resize), trace));
                Processed {
                    reply: Envelope {
                        schema_version: SCHEMA_VERSION,
                        request_id,
                        body: Some(reply),
                    }
                    .encode_to_vec(),
                    kind,
                    error: issue,
                    note,
                }
            }
            Err(fail) => {
                let mut failed = Processed::failed(
                    error_envelope(request_id, fail.code, fail.message),
                    kind,
                    fail.code,
                );
                // どのセッションが知られていなかったかを調べられるように、番号だけ添える（ADR 0034 §4.3）
                if fail.code == ErrorCode::UnknownSession {
                    failed.note = session.map(session_note);
                }
                failed
            }
        }
    }

    /// 返事と、ログに添える固定の語の注記（`Convert` のキャッシュの使われ方。ほかは None）
    pub(crate) fn dispatch(
        &self,
        body: envelope::Body,
    ) -> Result<(envelope::Body, Option<String>), Fail> {
        match body {
            envelope::Body::Convert(c) => self.convert(c),
            envelope::Body::Prefetch(p) => self.prefetch(p),
            // 行に足す注記がある要求（ADR 0034 §4.3、§7）
            envelope::Body::BeginSession(b) => {
                let note = diag::begin_session_note(&b);
                self.begin_session(b).map(|reply| (reply, Some(note)))
            }
            envelope::Body::Commit(c) => self.commit(c),
            envelope::Body::CommitSegments(c) => self.commit_segments(c),
            // 終わったセッションの番号を残す（集計がセッションの区切りに使う。ADR 0034 §5）
            envelope::Body::EndSession(e) => {
                let note = session_note(e.session_id);
                Ok((self.end_session(e), Some(note)))
            }
            other => self.dispatch_plain(other).map(|reply| (reply, None)),
        }
    }

    fn dispatch_plain(&self, body: envelope::Body) -> Result<envelope::Body, Fail> {
        match body {
            envelope::Body::Hello(_) => Ok(self.hello()),
            envelope::Body::Page(p) => self.page(p),
            envelope::Body::CommitText(c) => self.commit_text(c),
            envelope::Body::Cancel(c) => Ok(self.cancel(c)),
            envelope::Body::Shutdown(_) => Ok(self.shutdown()),
            // 文節の編集と再変換（ADR 0006。`segments.rs`）
            envelope::Body::SegmentPage(p) => self.segment_page(p),
            envelope::Body::ResizeSegment(r) => self.resize_segment(r),
            envelope::Body::Reconvert(r) => self.reconvert(r),
            // 管理の要求（ADR 0005 §8）
            envelope::Body::ListUserDict(_)
            | envelope::Body::AddUserDict(_)
            | envelope::Body::RemoveUserDict(_)
            | envelope::Body::LearningStatus(_)
            | envelope::Body::ResetLearning(_)
            | envelope::Body::ExportData(_)
            | envelope::Body::ImportData(_)
            | envelope::Body::GetProfile(_)
            | envelope::Body::SetProfile(_) => self.admin(body),
            // 絵文字のショートコードの表（ADR 0027 §9.2）
            envelope::Body::EmojiTable(r) => Ok(self.emoji_table(r)),
            // 英語のモードの郵便番号（ADR 0032 §9.1）
            envelope::Body::PostalLookup(r) => self.postal_lookup(r),
            // 診断ログ（ADR 0034 §2.2。`diag.rs`）。返事は常に Ack
            envelope::Body::Diag(d) => Ok(self.diag(&d)),
            // 返事の種類や Error を要求として受けたとき
            _ => Err(Fail::bad("unexpected message")),
        }
    }

    /// 絵文字の表の要求（ADR 0027 §9.2、§9.3）。読み込み済みの表（`data.rs` が起動時と見回りで読む）を返す。
    /// ファイルがなければ空の表（エラーにしない）。
    pub(crate) fn emoji_table(&self, _request: pb::EmojiTableRequest) -> envelope::Body {
        let reply = Arc::clone(&self.emoji.read().unwrap_or_else(|e| e.into_inner()));
        envelope::Body::EmojiTableReply((*reply).clone())
    }

    /// 郵便番号 → 住所の問い合わせ（ADR 0032 §9.1）。code は ASCII 数字 7 桁だけ（違えば BAD_REQUEST）。
    /// 表にない番号、表が未読み込みのときは空の配列。code はログに書かない（NFR-19）ので、エラーの文にも含めない。
    pub(crate) fn postal_lookup(
        &self,
        request: pb::PostalLookupRequest,
    ) -> Result<envelope::Body, Fail> {
        let code = request.code;
        if code.len() != POSTAL_CODE_DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Fail::bad("postal code must be 7 ASCII digits"));
        }
        let number: u32 = code
            .parse()
            .map_err(|_| Fail::bad("postal code must be 7 ASCII digits"))?;
        let stack = self.stack();
        let addresses = stack
            .postal
            .as_ref()
            .map(|table| {
                table
                    .by_code(number)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(envelope::Body::PostalLookupReply(pb::PostalLookupReply {
            addresses,
        }))
    }

    /// 文節の区切りの口。テストの上書きがなければ、Stack の Converter を呼ぶ
    pub(crate) fn seam(&self, stack: &Stack) -> Arc<dyn SegmentSeam> {
        if let Some(seam) = &self.seam_override {
            return Arc::clone(seam);
        }
        Arc::new(RealSeam(Arc::clone(&stack.converter)))
    }
}

/// 郵便番号の桁数（ADR 0032）
const POSTAL_CODE_DIGITS: usize = 7;

/// `data_dir` の下の `emoji.tsv`。辞書と同じ `dict/` に置く（ADR 0027 §9.3）
pub(crate) fn emoji_tsv_path_in(data_dir: &Path) -> PathBuf {
    UserDir::new(data_dir)
        .dict_dir()
        .join(ikari_core::EMOJI_TSV_FILE_NAME)
}

/// `data_dir` の下の `postal.tsv`。辞書と同じ `dict/` に置く（ADR 0032 §3）
pub(crate) fn postal_tsv_path_in(data_dir: &Path) -> PathBuf {
    UserDir::new(data_dir)
        .dict_dir()
        .join(ikari_core::POSTAL_TSV_FILE_NAME)
}
