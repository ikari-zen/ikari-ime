//! 置き場所の検査の規則（ADR 0067 §3.1。R1）。OS に依存しない（Mac でテストする）。
//!
//! 確かめること: インストール先とその下のどのファイル・フォルダにも、管理者・SYSTEM・TrustedInstaller 以外に
//! 書き込みを許す項目が無い。持ち主もこの 3 つのどれか（持ち主は ACL の変更を暗に許される）。
//! リンク（再解析点）は、行き先が利用者の場所でありうるので、それだけで失敗にする。
//!
//! 安全な側に倒す: 読めない記述子、知らない種類の許可の ACE、NULL の DACL は、すべて失敗。拒否の ACE は
//! 何も許さないので数えない（拒否で打ち消されていても、許可の ACE があれば失敗にする）。

/// SYSTEM
pub const SID_SYSTEM: &str = "S-1-5-18";
/// 組み込みの Administrators
pub const SID_ADMINISTRATORS: &str = "S-1-5-32-544";
/// TrustedInstaller（NT SERVICE\TrustedInstaller）
pub const SID_TRUSTED_INSTALLER: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// 書き込みを許すとみなす権利（ファイルとフォルダの両方の意味）。
/// FILE_WRITE_DATA（フォルダではファイルの追加）、FILE_APPEND_DATA（フォルダの追加）、FILE_WRITE_EA、
/// FILE_DELETE_CHILD、FILE_WRITE_ATTRIBUTES、DELETE、WRITE_DAC、WRITE_OWNER、GENERIC_ALL、GENERIC_WRITE
pub const WRITE_MASK: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0010
    | 0x0000_0040
    | 0x0000_0100
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000
    | 0x1000_0000
    | 0x4000_0000;

/// CREATOR OWNER（継承の先で、作った者の SID に置き換わる）
pub const SID_CREATOR_OWNER: &str = "S-1-3-0";

/// ACE の継承だけに使う印（INHERIT_ONLY_ACE）。この印の ACE は、その物自体には効かない
pub const INHERIT_ONLY: u8 = 0x08;

/// 書き込みを許してよい SID
const TRUSTED: &[&str] = &[SID_SYSTEM, SID_ADMINISTRATORS, SID_TRUSTED_INSTALLER];

/// DACL の 1 項目（OS 層が読んで作る）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ace {
    /// ACCESS_ALLOWED_ACE_TYPE
    Allowed { sid: String, mask: u32, flags: u8 },
    /// ACCESS_DENIED_ACE_TYPE（何も許さない）
    Denied,
    /// ほかの種類（オブジェクト、条件つき、など）。中身を解かないので、安全な側で失敗にする
    Other { ace_type: u8 },
}

/// 1 つのファイルかフォルダのセキュリティ記述子（OS 層が読んで作る）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// 持ち主の SID。読めなければ `None`
    pub owner: Option<String>,
    /// DACL。NULL の DACL（だれにでもすべてを許す）は `None`
    pub dacl: Option<Vec<Ace>>,
    /// リンク（再解析点）か
    pub reparse_point: bool,
}

/// 見つけた問題。表示は固定の語と SID だけ（パスは呼び出し側が添える）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    ReparsePoint,
    UnknownOwner,
    UntrustedOwner(String),
    NullDacl,
    UnknownAce(u8),
    WritableBy(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::ReparsePoint => f.write_str("reparse-point"),
            Problem::UnknownOwner => f.write_str("owner-unreadable"),
            Problem::UntrustedOwner(sid) => write!(f, "owner={sid}"),
            Problem::NullDacl => f.write_str("null-dacl"),
            Problem::UnknownAce(t) => write!(f, "ace-type={t}"),
            Problem::WritableBy(sid) => write!(f, "writable-by={sid}"),
        }
    }
}

fn trusted(sid: &str) -> bool {
    TRUSTED.iter().any(|t| t.eq_ignore_ascii_case(sid))
}

/// 1 つの記述子の問題をすべて返す（空なら合格）。
pub fn problems(descriptor: &Descriptor) -> Vec<Problem> {
    let mut found = Vec::new();
    if descriptor.reparse_point {
        found.push(Problem::ReparsePoint);
    }
    match &descriptor.owner {
        None => found.push(Problem::UnknownOwner),
        Some(sid) if !trusted(sid) => found.push(Problem::UntrustedOwner(sid.clone())),
        Some(_) => {}
    }
    let Some(dacl) = &descriptor.dacl else {
        found.push(Problem::NullDacl);
        return found;
    };
    for ace in dacl {
        match ace {
            Ace::Denied => {}
            Ace::Other { ace_type } => found.push(Problem::UnknownAce(*ace_type)),
            Ace::Allowed { sid, mask, flags } => {
                // 継承だけの ACE を見逃してよいのは、信頼する SID と CREATOR OWNER だけ
                // （それ以外の SID の継承は、このあと作る子に書ける権利を渡す）
                if flags & INHERIT_ONLY != 0
                    && (trusted(sid) || sid.eq_ignore_ascii_case(SID_CREATOR_OWNER))
                {
                    continue;
                }
                if mask & WRITE_MASK != 0 && !trusted(sid) {
                    found.push(Problem::WritableBy(sid.clone()));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Program Files の下の既定（継承したまま）の形
    fn program_files_like() -> Descriptor {
        Descriptor {
            owner: Some(SID_ADMINISTRATORS.into()),
            dacl: Some(vec![
                Ace::Allowed {
                    sid: SID_TRUSTED_INSTALLER.into(),
                    mask: 0x001F_01FF,
                    flags: 0,
                },
                Ace::Allowed {
                    sid: SID_SYSTEM.into(),
                    mask: 0x001F_01FF,
                    flags: 0x10,
                },
                Ace::Allowed {
                    sid: SID_ADMINISTRATORS.into(),
                    mask: 0x001F_01FF,
                    flags: 0x10,
                },
                // Users: 読み取りと実行
                Ace::Allowed {
                    sid: "S-1-5-32-545".into(),
                    mask: 0x0012_00A9,
                    flags: 0x10,
                },
                // CREATOR OWNER: 継承だけ（この物自体には効かない）
                Ace::Allowed {
                    sid: "S-1-3-0".into(),
                    mask: 0x1000_0000,
                    flags: 0x0B,
                },
                // ALL APPLICATION PACKAGES: 読み取りと実行（ストアのアプリが TIP を読むのに要る）
                Ace::Allowed {
                    sid: "S-1-15-2-1".into(),
                    mask: 0x0012_00A9,
                    flags: 0x10,
                },
            ]),
            reparse_point: false,
        }
    }

    #[test]
    fn the_inherited_program_files_acl_passes() {
        assert_eq!(problems(&program_files_like()), vec![]);
    }

    #[test]
    fn a_user_with_write_fails() {
        let mut d = program_files_like();
        d.dacl.as_mut().unwrap().push(Ace::Allowed {
            sid: "S-1-5-32-545".into(),
            mask: 0x0000_0002,
            flags: 0,
        });
        assert_eq!(
            problems(&d),
            vec![Problem::WritableBy("S-1-5-32-545".into())]
        );
    }

    #[test]
    fn every_write_right_counts() {
        for bit in [
            0x2u32,
            0x4,
            0x10,
            0x40,
            0x100,
            0x1_0000,
            0x4_0000,
            0x8_0000,
            0x1000_0000,
            0x4000_0000,
        ] {
            let d = Descriptor {
                owner: Some(SID_SYSTEM.into()),
                dacl: Some(vec![Ace::Allowed {
                    sid: "S-1-1-0".into(),
                    mask: bit,
                    flags: 0,
                }]),
                reparse_point: false,
            };
            assert_eq!(
                problems(&d),
                vec![Problem::WritableBy("S-1-1-0".into())],
                "{bit:#x}"
            );
        }
    }

    #[test]
    fn read_and_execute_only_passes_for_anyone() {
        let d = Descriptor {
            owner: Some(SID_TRUSTED_INSTALLER.into()),
            dacl: Some(vec![Ace::Allowed {
                sid: "S-1-1-0".into(),
                mask: 0x0012_00A9,
                flags: 0,
            }]),
            reparse_point: false,
        };
        assert_eq!(problems(&d), vec![]);
    }

    #[test]
    fn an_owner_outside_the_trusted_set_fails() {
        // 管理者の利用者自身が持ち主だと、その利用者のふつうの権限のプロセスが ACL を変えられる
        let mut d = program_files_like();
        d.owner = Some("S-1-5-21-1-2-3-1001".into());
        assert_eq!(
            problems(&d),
            vec![Problem::UntrustedOwner("S-1-5-21-1-2-3-1001".into())]
        );
        d.owner = None;
        assert_eq!(problems(&d), vec![Problem::UnknownOwner]);
    }

    #[test]
    fn null_dacl_unknown_aces_and_links_fail() {
        let mut d = program_files_like();
        d.dacl = None;
        assert_eq!(problems(&d), vec![Problem::NullDacl]);

        let mut d = program_files_like();
        d.dacl.as_mut().unwrap().push(Ace::Other { ace_type: 5 });
        assert_eq!(problems(&d), vec![Problem::UnknownAce(5)]);

        let mut d = program_files_like();
        d.reparse_point = true;
        assert_eq!(problems(&d), vec![Problem::ReparsePoint]);
    }

    #[test]
    fn deny_aces_do_not_excuse_an_allow() {
        let mut d = program_files_like();
        let dacl = d.dacl.as_mut().unwrap();
        dacl.insert(0, Ace::Denied);
        dacl.push(Ace::Allowed {
            sid: "S-1-5-11".into(),
            mask: 0x4000_0000,
            flags: 0,
        });
        assert_eq!(problems(&d), vec![Problem::WritableBy("S-1-5-11".into())]);
    }

    #[test]
    fn inherit_only_write_for_an_untrusted_sid_fails() {
        let mut d = program_files_like();
        d.dacl.as_mut().unwrap().push(Ace::Allowed {
            sid: "S-1-5-32-545".into(),
            mask: 0x1000_0000,
            flags: 0x0B,
        });
        assert_eq!(
            problems(&d),
            vec![Problem::WritableBy("S-1-5-32-545".into())]
        );
    }

    #[test]
    fn inherit_only_write_for_trusted_sids_and_creator_owner_passes() {
        for sid in [
            SID_SYSTEM,
            SID_ADMINISTRATORS,
            SID_TRUSTED_INSTALLER,
            "S-1-3-0",
        ] {
            let d = Descriptor {
                owner: Some(SID_SYSTEM.into()),
                dacl: Some(vec![Ace::Allowed {
                    sid: sid.into(),
                    mask: 0x1000_0000,
                    flags: 0x0B,
                }]),
                reparse_point: false,
            };
            assert_eq!(problems(&d), vec![], "{sid}");
        }
    }

    #[test]
    fn inherit_only_read_for_an_untrusted_sid_still_passes() {
        let d = Descriptor {
            owner: Some(SID_SYSTEM.into()),
            dacl: Some(vec![Ace::Allowed {
                sid: "S-1-1-0".into(),
                mask: 0x0012_00A9,
                flags: 0x0B,
            }]),
            reparse_point: false,
        };
        assert_eq!(problems(&d), vec![]);
    }

    #[test]
    fn trusted_sids_compare_without_case() {
        let d = Descriptor {
            owner: Some("s-1-5-18".into()),
            dacl: Some(vec![]),
            reparse_point: false,
        };
        assert_eq!(problems(&d), vec![]);
    }
}
