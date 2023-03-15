use std::{borrow::Cow, fmt::Display, path::Path, str::Utf8Error};

use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::CompileError;

fn starts_with_windows_drive(p: &str) -> bool {
    p.chars().next().filter(char::is_ascii_uppercase).is_some() && p[1..].starts_with(":\\")
}

pub(crate) fn to_url(s: &str) -> Result<Url, CompileError> {
    debug_assert!(!s.contains('#'));

    // note: windows drive letter is treated as url scheme by url parser
    if std::env::consts::OS == "windows" && starts_with_windows_drive(s) {
        return Url::from_file_path(s)
            .map_err(|_| CompileError::Bug(format!("failed to convert {s} into url").into()));
    }
    match Url::parse(s) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let path = Path::new(s);
            let path = path
                .canonicalize()
                .map_err(|e| CompileError::LoadUrlError {
                    url: s.to_owned(),
                    src: e.into(),
                })?;
            Url::from_file_path(path)
                .map_err(|_| CompileError::Bug(format!("failed to convert {s} into url").into()))
        }
        Err(e) => Err(CompileError::ParseUrlError {
            url: s.to_owned(),
            src: e.into(),
        }),
    }
}

/// returns single-quoted string
pub(crate) fn quote<T>(s: &T) -> String
where
    T: AsRef<str> + std::fmt::Debug + ?Sized,
{
    let s = format!("{s:?}")
        .replace(r#"\""#, "\"")
        .replace('\'', r#"\'"#);
    format!("'{}'", &s[1..s.len() - 1])
}

pub(crate) fn join_iter<T>(iterable: T, sep: &str) -> String
where
    T: IntoIterator,
    T::Item: Display,
{
    iterable
        .into_iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

fn path_escape(s: &str) -> String {
    url_escape::encode_path(s).into_owned()
}

pub(crate) fn escape(token: &str) -> String {
    path_escape(&token.replace('~', "~0").replace('/', "~1"))
}

pub(crate) fn path_unescape(s: &str) -> Result<String, Utf8Error> {
    Ok(percent_decode_str(s).decode_utf8()?.into_owned())
}

pub(crate) fn unescape(mut token: &str) -> Result<Cow<str>, ()> {
    let Some(mut tilde) = token.find('~') else {
        return Ok(Cow::Borrowed(token));
    };
    let mut s = String::with_capacity(token.len());
    loop {
        s.push_str(&token[..tilde]);
        token = &token[tilde + 1..];
        match token.chars().next() {
            Some('1') => s.push('/'),
            Some('0') => s.push('~'),
            _ => return Err(()),
        }
        token = &token[1..];
        let Some(i) = token.find('~') else {
            s.push_str(token);
            break;
        };
        tilde = i;
    }
    Ok(Cow::Owned(s))
}

pub(crate) struct Fragment<'a>(&'a str);

impl<'a> Fragment<'a> {
    pub(crate) fn as_str(&self) -> &str {
        self.0
    }

    fn is_json_pointer(&self) -> bool {
        self.0.is_empty()
            || self.0.starts_with('/')
            || self.0.starts_with("%2F")
            || self.0.starts_with("%2f")
    }

    pub(crate) fn is_anchor(&self) -> bool {
        !self.is_json_pointer()
    }

    pub(crate) fn decode(&self) -> Result<Cow<str>, Utf8Error> {
        return percent_decode_str(self.0).decode_utf8();
    }

    pub(crate) fn to_anchor(&self) -> Result<Option<Cow<str>>, Utf8Error> {
        if self.is_json_pointer() {
            Ok(None) // json-pointer
        } else {
            Ok(Some(self.decode()?)) // anchor
        }
    }
}

impl<'a> Display for Fragment<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub(crate) fn split(url: &str) -> (&str, Fragment) {
    if let Some(i) = url.find('#') {
        (&url[..i], Fragment(&url[i + 1..]))
    } else {
        (url, Fragment(""))
    }
}

/// serde_json treats 0 and 0.0 not equal. so we cannot simply use v1==v2
pub(crate) fn equals(v1: &Value, v2: &Value) -> bool {
    match (v1, v2) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(b1), Value::Bool(b2)) => b1 == b2,
        (Value::Number(n1), Value::Number(n2)) => {
            if let (Some(n1), Some(n2)) = (n1.as_u64(), n2.as_u64()) {
                return n1 == n2;
            }
            if let (Some(n1), Some(n2)) = (n1.as_i64(), n2.as_i64()) {
                return n1 == n2;
            }
            if let (Some(n1), Some(n2)) = (n1.as_f64(), n2.as_f64()) {
                return n1 == n2;
            }
            false
        }
        (Value::String(s1), Value::String(s2)) => s1 == s2,
        (Value::Array(arr1), Value::Array(arr2)) => {
            if arr1.len() != arr2.len() {
                return false;
            }
            arr1.iter().zip(arr2).all(|(e1, e2)| equals(e1, e2))
        }
        (Value::Object(obj1), Value::Object(obj2)) => {
            if obj1.len() != obj2.len() {
                return false;
            }
            for (k1, v1) in obj1 {
                if let Some(v2) = obj2.get(k1) {
                    if !equals(v1, v2) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn to_strings(v: &Value) -> Vec<String> {
    if let Value::Array(a) = v {
        a.iter()
            .filter_map(|t| {
                if let Value::String(t) = t {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    }
}

pub(crate) fn write_json_to_fmt<T>(
    f: &mut std::fmt::Formatter,
    value: &T,
) -> Result<(), std::fmt::Error>
where
    T: ?Sized + Serialize,
{
    let s = if f.alternate() {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    };
    f.write_str(s.map_err(|_| std::fmt::Error)?.as_str())
}

/*
// Loc --

pub(crate) enum Loc<'a> {
    Abs(&'a str),
    Relative(usize, &'a str),
}

impl<'a> Loc<'a> {
    pub(crate) fn locate(from: &str, to: &'a str) -> Loc<'a> {
        if let Some(path) = to.strip_prefix(from) {
            return Self::Relative(0, path);
        }
        let (_, path) = split(from); // todo: fragment misuse
        if let Some(mut i) = path.0.rfind('/') {
            i = from.len() - 1 - (path.0.len() - 1 - i);
            return match Self::locate(&from[..i], to) {
                Self::Relative(i, ptr) => Self::Relative(i + 1, ptr),
                loc => loc,
            };
        }
        Self::Abs(to)
    }
}

impl<'a> Display for Loc<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abs(loc) => write!(f, "{loc}"),
            Self::Relative(i, path) => write!(f, "{i}{path}"),
        }
    }
}
*/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote() {
        assert_eq!(quote(r#"abc"def'ghi"#), r#"'abc"def\'ghi'"#);
    }

    #[test]
    fn test_path_escape() {
        let tests = [
            ("my%2Fcool+blog&about,stuff", "my%2Fcool+blog&about,stuff"),
            ("a\nb", "a%0Ab"),
        ];
        for (raw, want) in tests {
            assert_eq!(path_escape(raw), want);
        }
    }

    #[test]
    fn test_path_unescape() {
        assert_eq!(
            path_unescape("my%2Fcool+blog&about,stuff").unwrap(),
            "my/cool+blog&about,stuff",
        );
    }

    #[test]
    fn test_fragment_to_anchor() {
        assert_eq!(Fragment("").to_anchor(), Ok(None));
        assert_eq!(Fragment("/a/b").to_anchor(), Ok(None));
        assert_eq!(Fragment("abcd").to_anchor(), Ok(Some(Cow::from("abcd"))));
        assert_eq!(
            Fragment("%61%62%63%64").to_anchor(),
            Ok(Some(Cow::from("abcd")))
        );
    }

    #[test]
    fn test_equals() {
        let tests = [["1.0", "1"], ["-1.0", "-1"]];
        for [a, b] in tests {
            let a = serde_json::from_str(a).unwrap();
            let b = serde_json::from_str(b).unwrap();
            assert!(equals(&a, &b));
        }
    }
}
