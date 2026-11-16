//! インストール先の木をたどって、どの物も `acl::problems` を満たすか調べる（ADR 0067 §3.1）。
//! 記述子を読むのは呼び出し側（Windows は `os::read_descriptor`、テストは作り物）。たどる順と、リンクの扱いは
//! ここで決める（OS に依存しない。Mac でテストする）。

use std::io;
use std::path::{Path, PathBuf};

use crate::acl::{Descriptor, Problem, problems};

/// 1 つの物の結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    /// 読めなかった（安全な側で失敗に数える）か、規則の問題
    pub problems: Vec<String>,
}

/// たどった結果
#[derive(Debug, Default)]
pub struct Report {
    /// 調べた物の数
    pub checked: usize,
    /// 問題のあった物
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.findings.is_empty()
    }
}

/// `root` とその下をすべて調べる。リンク（`read` が `reparse_point` を返したもの、または std がリンクと見るもの）
/// の先はたどらない（リンクそのものが問題として数えられる）。
pub fn check(root: &Path, read: &dyn Fn(&Path) -> io::Result<Descriptor>) -> Report {
    let mut report = Report::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        report.checked += 1;
        let descriptor = match read(&path) {
            Ok(d) => d,
            Err(e) => {
                report.findings.push(Finding {
                    path,
                    problems: vec![format!("unreadable ({})", e.kind())],
                });
                continue;
            }
        };
        let found: Vec<String> = problems(&descriptor)
            .iter()
            .map(Problem::to_string)
            .collect();
        let is_link = descriptor.reparse_point;
        if !found.is_empty() {
            report.findings.push(Finding {
                path: path.clone(),
                problems: found,
            });
        }
        if is_link {
            continue;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                report.findings.push(Finding {
                    path,
                    problems: vec![format!("unreadable ({})", e.kind())],
                });
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            report.findings.push(Finding {
                path,
                problems: vec![Problem::ReparsePoint.to_string()],
            });
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => pending.push(entry.path()),
                        Err(e) => report.findings.push(Finding {
                            path: path.clone(),
                            problems: vec![format!("unlistable ({})", e.kind())],
                        }),
                    }
                }
            }
            Err(e) => report.findings.push(Finding {
                path,
                problems: vec![format!("unlistable ({})", e.kind())],
            }),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::{Ace, SID_ADMINISTRATORS, SID_SYSTEM};

    fn good() -> Descriptor {
        Descriptor {
            owner: Some(SID_ADMINISTRATORS.into()),
            dacl: Some(vec![Ace::Allowed {
                sid: SID_SYSTEM.into(),
                mask: 0x001F_01FF,
                flags: 0,
            }]),
            reparse_point: false,
        }
    }

    #[test]
    fn walks_every_file_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tip/0.1.0/resources")).unwrap();
        std::fs::write(dir.path().join("ikari-engine-host.exe"), b"").unwrap();
        std::fs::write(dir.path().join("tip/0.1.0/ikari_tip.dll"), b"").unwrap();
        let report = check(dir.path(), &|_| Ok(good()));
        assert!(report.ok());
        // root、exe、tip、0.1.0、resources、dll
        assert_eq!(report.checked, 6);
    }

    #[test]
    fn one_bad_file_fails_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tip")).unwrap();
        let bad = dir.path().join("tip/ikari_tip.dll");
        std::fs::write(&bad, b"").unwrap();
        let report = check(dir.path(), &|p| {
            let mut d = good();
            if p == bad {
                d.dacl.as_mut().unwrap().push(Ace::Allowed {
                    sid: "S-1-5-11".into(),
                    mask: 0x2,
                    flags: 0,
                });
            }
            Ok(d)
        });
        assert!(!report.ok());
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].path, bad);
        assert_eq!(report.findings[0].problems, vec!["writable-by=S-1-5-11"]);
    }

    #[test]
    fn unreadable_counts_as_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let report = check(dir.path(), &|_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(!report.ok());
    }

    #[test]
    fn links_are_reported_and_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("tip");
        std::fs::create_dir(&link).unwrap();
        std::fs::write(link.join("inner.dll"), b"").unwrap();
        let report = check(dir.path(), &|p| {
            let mut d = good();
            d.reparse_point = p == link;
            Ok(d)
        });
        assert_eq!(report.checked, 2, "the inside of a link is not walked");
        assert_eq!(report.findings[0].problems, vec!["reparse-point"]);
    }

    #[cfg(unix)]
    #[test]
    fn std_symlinks_are_reported_too() {
        let dir = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(target.path(), dir.path().join("tip")).unwrap();
        let report = check(dir.path(), &|_| Ok(good()));
        assert!(!report.ok());
        assert_eq!(report.findings[0].problems, vec!["reparse-point"]);
    }
}
