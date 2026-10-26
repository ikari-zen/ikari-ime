//! `*Config::from_kv` の共通部品（自分の接頭辞のキーだけを読む）。

use std::collections::BTreeMap;
use std::str::FromStr;

use super::RankerConfigError;

/// `from_kv` で自分のキー（`<prefix>.<name>`）だけを取り出す。
/// 他の Ranker のキー（別の接頭辞）は無視し、自分の接頭辞で未知の名前なら `RankerConfigError`。
pub(crate) struct ScopedKv<'a> {
    prefix: &'static str,
    values: BTreeMap<&'a str, &'a str>,
}

impl<'a> ScopedKv<'a> {
    pub(crate) fn new(
        prefix: &'static str,
        known: &[&str],
        kv: &'a BTreeMap<String, String>,
    ) -> Result<Self, RankerConfigError> {
        let mut values = BTreeMap::new();
        for (key, value) in kv {
            let Some(name) = key
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('.'))
            else {
                continue;
            };
            if !known.contains(&name) {
                return Err(RankerConfigError {
                    key: key.clone(),
                    message: format!("unknown key (expected one of: {})", known.join(", ")),
                });
            }
            values.insert(name, value.as_str());
        }
        Ok(Self { prefix, values })
    }

    /// 0 以上の値として読む。指定がなければ `default`。
    pub(crate) fn get<T>(&self, name: &str, default: T) -> Result<T, RankerConfigError>
    where
        T: FromStr + PartialOrd + Default,
    {
        let Some(raw) = self.values.get(name) else {
            return Ok(default);
        };
        let err = |message: String| RankerConfigError {
            key: format!("{}.{name}", self.prefix),
            message,
        };
        let value: T = raw
            .trim()
            .parse()
            .map_err(|_| err(format!("`{raw}` is not a valid number")))?;
        // NaN は比較が偽になるのでここで弾かれる
        if value >= T::default() {
            Ok(value)
        } else {
            Err(err(format!("`{raw}` must not be negative")))
        }
    }

    /// 文字列として読む（前後の空白は除く）。指定がなければ `default`。
    pub(crate) fn get_arc_str(
        &self,
        name: &str,
        default: &std::sync::Arc<str>,
    ) -> std::sync::Arc<str> {
        match self.values.get(name) {
            Some(raw) => std::sync::Arc::from(raw.trim()),
            None => std::sync::Arc::clone(default),
        }
    }

    /// 有限の 0 以上の小数として読む。
    pub(super) fn get_float(&self, name: &str, default: f32) -> Result<f32, RankerConfigError> {
        let value = self.get(name, default)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(RankerConfigError {
                key: format!("{}.{name}", self.prefix),
                message: "must be finite".to_owned(),
            })
        }
    }
}
