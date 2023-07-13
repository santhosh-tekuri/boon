use std::{borrow::Cow, env, fmt::Display, hash::Hash, hash::Hasher, path::Path, str::Utf8Error};

use ahash::AHasher;
use percent_encoding::percent_decode_str;
use serde_json::Value;
use url::Url;

use crate::CompileError;

pub(crate) fn is_integer(v: &Value) -> bool {
    match v {
        Value::Number(n) => {
            n.is_i64() || n.is_u64() || n.as_f64().filter(|n| n.fract() == 0.0).is_some()
        }
        _ => false,
    }
}

fn starts_with_windows_drive(p: &str) -> bool {
    p.chars().next().filter(char::is_ascii_uppercase).is_some() && p[1..].starts_with(":\\")
}

#[cfg(feature = "resolve-file")]
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
            let mut path = Path::new(s);
            let tmp;
            if !path.is_absolute() {
                tmp = env::current_dir()
                    .map_err(|e| CompileError::ParseUrlError {
                        url: s.to_owned(),
                        src: e.into(),
                    })?
                    .join(path);
                path = tmp.as_path();
            }
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

pub(crate) fn escape(token: &str) -> Cow<str> {
    const SPECIAL: [char; 2] = ['~', '/'];
    if token.contains(SPECIAL) {
        token.replace('~', "~0").replace('/', "~1").into()
    } else {
        token.into()
    }
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
    pub(crate) fn as_str(&self) -> &'a str {
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

// HashedValue --

// Based on implementation proposed by Sven Marnach:
// https://stackoverflow.com/questions/60882381/what-is-the-fastest-correct-way-to-detect-that-there-are-no-duplicates-in-a-json
#[derive(PartialEq)]
pub(crate) struct HashedValue<'a>(pub(crate) &'a Value);

impl Eq for HashedValue<'_> {}

impl Hash for HashedValue<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            Value::Null => state.write_u32(3_221_225_473), // chosen randomly
            Value::Bool(ref b) => b.hash(state),
            Value::Number(ref num) => {
                if let Some(num) = num.as_u64() {
                    num.hash(state);
                } else if let Some(num) = num.as_i64() {
                    num.hash(state);
                } else if let Some(num) = num.as_f64() {
                    num.to_bits().hash(state)
                }
            }
            Value::String(ref str) => str.hash(state),
            Value::Array(ref arr) => {
                for item in arr {
                    HashedValue(item).hash(state);
                }
            }
            Value::Object(ref obj) => {
                let mut hash = 0;
                for (pname, pvalue) in obj {
                    // We have no way of building a new hasher of type `H`, so we
                    // hardcode using the default hasher of a hash map.
                    let mut hasher = AHasher::default();
                    pname.hash(&mut hasher);
                    HashedValue(pvalue).hash(&mut hasher);
                    hash ^= hasher.finish();
                }
                state.write_u64(hash);
            }
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
