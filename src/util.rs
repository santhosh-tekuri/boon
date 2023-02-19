use std::{borrow::Cow, fmt::Display, str::Utf8Error};

use percent_encoding::percent_decode_str;
use serde_json::Value;

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

pub(crate) fn unescape(token: &str) -> Result<String, Utf8Error> {
    path_unescape(&token.replace("~1", "/").replace("~0", "~"))
}

pub(crate) fn fragment_to_anchor(fragment: &str) -> Result<Option<Cow<str>>, Utf8Error> {
    if fragment.is_empty() || fragment.starts_with('/') {
        Ok(None) // json-pointer
    } else {
        Ok(Some(percent_decode_str(fragment).decode_utf8()?)) // anchor
    }
}

pub(crate) fn split(url: &str) -> (&str, &str) {
    if let Some(i) = url.find('#') {
        (&url[..i], &url[i + 1..])
    } else {
        (url, "")
    }
}

pub(crate) fn ptr_tokens(ptr: &str) -> impl Iterator<Item = Result<String, Utf8Error>> + '_ {
    ptr.split('/').skip(1).map(unescape)
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
        let (_, path) = split(from);
        if let Some(mut i) = path.rfind('/') {
            i = from.len() - 1 - (path.len() - 1 - i);
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
        assert_eq!(fragment_to_anchor(""), Ok(None));
        assert_eq!(fragment_to_anchor("/a/b"), Ok(None));
        assert_eq!(fragment_to_anchor("abcd"), Ok(Some(Cow::from("abcd"))));
        assert_eq!(
            fragment_to_anchor("%61%62%63%64"),
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
