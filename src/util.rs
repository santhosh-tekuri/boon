use std::{
    borrow::{Borrow, Cow},
    env,
    fmt::Display,
    hash::Hash,
    hash::Hasher,
    str::FromStr,
};

use ahash::{AHashMap, AHasher};
use percent_encoding::{percent_decode_str, AsciiSet, CONTROLS};
use serde_json::Value;
use url::Url;

use crate::CompileError;

// --

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct JsonPointer(pub(crate) String);

impl JsonPointer {
    pub(crate) fn escape(token: &str) -> Cow<str> {
        const SPECIAL: [char; 2] = ['~', '/'];
        if token.contains(SPECIAL) {
            token.replace('~', "~0").replace('/', "~1").into()
        } else {
            token.into()
        }
    }

    pub(crate) fn unescape(mut tok: &str) -> Result<Cow<str>, ()> {
        let Some(mut tilde) = tok.find('~') else {
            return Ok(Cow::Borrowed(tok));
        };
        let mut s = String::with_capacity(tok.len());
        loop {
            s.push_str(&tok[..tilde]);
            tok = &tok[tilde + 1..];
            match tok.chars().next() {
                Some('1') => s.push('/'),
                Some('0') => s.push('~'),
                _ => return Err(()),
            }
            tok = &tok[1..];
            let Some(i) = tok.find('~') else {
                s.push_str(tok);
                break;
            };
            tilde = i;
        }
        Ok(Cow::Owned(s))
    }

    pub(crate) fn lookup<'a>(
        &self,
        mut v: &'a Value,
        v_url: &Url,
    ) -> Result<&'a Value, CompileError> {
        for tok in self.0.split('/').skip(1) {
            let Ok(tok) = Self::unescape(tok) else {
                let loc = UrlFrag::format(v_url, self.as_str());
                return Err(CompileError::InvalidJsonPointer(loc));
            };
            match v {
                Value::Object(obj) => {
                    if let Some(pvalue) = obj.get(tok.as_ref()) {
                        v = pvalue;
                        continue;
                    }
                }
                Value::Array(arr) => {
                    if let Ok(i) = usize::from_str(tok.as_ref()) {
                        if let Some(item) = arr.get(i) {
                            v = item;
                            continue;
                        }
                    };
                }
                _ => {}
            }
            let loc = UrlFrag::format(v_url, self.as_str());
            return Err(CompileError::JsonPointerNotFound(loc));
        }
        Ok(v)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn concat(&self, next: &Self) -> Self {
        JsonPointer(format!("{}{}", self.0, next.0))
    }

    pub(crate) fn append(&self, tok: &str) -> Self {
        Self(format!("{}/{}", self, Self::escape(tok)))
    }

    pub(crate) fn append2(&self, tok1: &str, tok2: &str) -> Self {
        Self(format!(
            "{}/{}/{}",
            self,
            Self::escape(tok1),
            Self::escape(tok2)
        ))
    }
}

impl Display for JsonPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Borrow<str> for JsonPointer {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for JsonPointer {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

// --

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct Anchor(pub(crate) String);

impl Display for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Borrow<str> for Anchor {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Anchor {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

// --
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum Fragment {
    Anchor(Anchor),
    JsonPointer(JsonPointer),
}

impl Fragment {
    pub(crate) fn split(s: &str) -> Result<(&str, Fragment), CompileError> {
        let (u, frag) = split(s);
        let frag = percent_decode_str(frag)
            .decode_utf8()
            .map_err(|src| CompileError::ParseUrlError {
                url: s.to_string(),
                src: src.into(),
            })?
            .to_string();
        let frag = if frag.is_empty() || frag.starts_with('/') {
            Fragment::JsonPointer(JsonPointer(frag))
        } else {
            Fragment::Anchor(Anchor(frag))
        };
        Ok((u, frag))
    }

    pub(crate) fn encode(frag: &str) -> String {
        // https://url.spec.whatwg.org/#fragment-percent-encode-set
        const FRAGMENT: &AsciiSet = &CONTROLS
            .add(b'%')
            .add(b' ')
            .add(b'"')
            .add(b'<')
            .add(b'>')
            .add(b'`');
        percent_encoding::utf8_percent_encode(frag, FRAGMENT).to_string()
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Fragment::Anchor(s) => &s.0,
            Fragment::JsonPointer(s) => &s.0,
        }
    }
}

// --

#[derive(Clone)]
pub(crate) struct UrlFrag {
    pub(crate) url: Url,
    pub(crate) frag: Fragment,
}

impl UrlFrag {
    pub(crate) fn absolute(input: &str) -> Result<UrlFrag, CompileError> {
        let (u, frag) = Fragment::split(input)?;

        // note: windows drive letter is treated as url scheme by url parser
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::consts::OS == "windows" && starts_with_windows_drive(u) {
            let url = Url::from_file_path(u)
                .map_err(|_| CompileError::Bug(format!("failed to convert {u} into url").into()))?;
            return Ok(UrlFrag { url, frag });
        }

        match Url::parse(u) {
            Ok(url) => Ok(UrlFrag { url, frag }),
            #[cfg(not(target_arch = "wasm32"))]
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                // TODO(unstable): replace with `path::absolute` once it is stabilized
                use std::path::Path;
                let mut path = Path::new(u);
                let tmp;
                if !path.is_absolute() {
                    tmp = env::current_dir()
                        .map_err(|e| CompileError::ParseUrlError {
                            url: u.to_owned(),
                            src: e.into(),
                        })?
                        .join(path);
                    path = tmp.as_path();
                }

                let url = Url::from_file_path(path).map_err(|_| {
                    CompileError::Bug(format!("failed to convert {u} into url").into())
                })?;
                Ok(UrlFrag { url, frag })
            }
            Err(e) => Err(CompileError::ParseUrlError {
                url: u.to_owned(),
                src: e.into(),
            }),
        }
    }

    pub(crate) fn join(url: &Url, input: &str) -> Result<UrlFrag, CompileError> {
        let (input, frag) = Fragment::split(input)?;
        if input.is_empty() {
            return Ok(UrlFrag {
                url: url.clone(),
                frag,
            });
        }
        let url = url.join(input).map_err(|e| CompileError::ParseUrlError {
            url: input.to_string(),
            src: e.into(),
        })?;

        Ok(UrlFrag { url, frag })
    }

    pub(crate) fn format(url: &Url, frag: &str) -> String {
        if frag.is_empty() {
            url.to_string()
        } else {
            format!("{}#{}", url, Fragment::encode(frag))
        }
    }
}

impl Display for UrlFrag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.url, Fragment::encode(self.frag.as_str()))
    }
}

// --

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct UrlPtr {
    pub(crate) url: Url,
    pub(crate) ptr: JsonPointer,
}

impl UrlPtr {
    pub(crate) fn lookup<'a>(&self, doc: &'a Value) -> Result<&'a Value, CompileError> {
        self.ptr.lookup(doc, &self.url)
    }

    pub(crate) fn format(&self, tok: &str) -> String {
        format!(
            "{}#{}/{}",
            self.url,
            Fragment::encode(self.ptr.as_str()),
            Fragment::encode(JsonPointer::escape(tok).as_ref()),
        )
    }
}

impl Display for UrlPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.url, Fragment::encode(self.ptr.as_str()))
    }
}

// --

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

/// returns single-quoted string
pub(crate) fn quote<T>(s: &T) -> String
where
    T: AsRef<str> + std::fmt::Debug + ?Sized,
{
    let s = format!("{s:?}").replace(r#"\""#, "\"").replace('\'', r"\'");
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
    JsonPointer::escape(token)
}

pub(crate) fn split(url: &str) -> (&str, &str) {
    if let Some(i) = url.find('#') {
        (&url[..i], &url[i + 1..])
    } else {
        (url, "")
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

pub(crate) fn duplicates(arr: &Vec<Value>) -> Option<(usize, usize)> {
    match arr.as_slice() {
        [e0, e1] => {
            if equals(e0, e1) {
                return Some((0, 1));
            }
        }
        [e0, e1, e2] => {
            if equals(e0, e1) {
                return Some((0, 1));
            } else if equals(e0, e2) {
                return Some((0, 2));
            } else if equals(e1, e2) {
                return Some((1, 2));
            }
        }
        _ => {
            let len = arr.len();
            if len <= 20 {
                for i in 0..len - 1 {
                    for j in i + 1..len {
                        if equals(&arr[i], &arr[j]) {
                            return Some((i, j));
                        }
                    }
                }
            } else {
                let mut seen = AHashMap::with_capacity(len);
                for (i, item) in arr.iter().enumerate() {
                    if let Some(j) = seen.insert(HashedValue(item), i) {
                        return Some((j, i));
                    }
                }
            }
        }
    }
    None
}

// HashedValue --

// Based on implementation proposed by Sven Marnach:
// https://stackoverflow.com/questions/60882381/what-is-the-fastest-correct-way-to-detect-that-there-are-no-duplicates-in-a-json
pub(crate) struct HashedValue<'a>(pub(crate) &'a Value);

impl PartialEq for HashedValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        equals(self.0, other.0)
    }
}

impl Eq for HashedValue<'_> {}

impl Hash for HashedValue<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            Value::Null => state.write_u32(3_221_225_473), // chosen randomly
            Value::Bool(ref b) => b.hash(state),
            Value::Number(ref num) => {
                if let Some(num) = num.as_f64() {
                    num.to_bits().hash(state);
                } else if let Some(num) = num.as_u64() {
                    num.hash(state);
                } else if let Some(num) = num.as_i64() {
                    num.hash(state);
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

    use ahash::AHashMap;
    use serde_json::json;

    use super::*;

    #[test]
    fn test_quote() {
        assert_eq!(quote(r#"abc"def'ghi"#), r#"'abc"def\'ghi'"#);
    }

    #[test]
    fn test_fragment_split() {
        let tests = [
            ("#", Fragment::JsonPointer("".into())),
            ("#/a/b", Fragment::JsonPointer("/a/b".into())),
            ("#abcd", Fragment::Anchor("abcd".into())),
            ("#%61%62%63%64", Fragment::Anchor("abcd".into())),
            (
                "#%2F%61%62%63%64%2fef",
                Fragment::JsonPointer("/abcd/ef".into()),
            ), // '/' is encoded
            ("#abcd+ef", Fragment::Anchor("abcd+ef".into())), // '+' should not traslate to space
        ];
        for test in tests {
            let (_, got) = Fragment::split(test.0).unwrap();
            assert_eq!(got, test.1, "Fragment::split({:?})", test.0);
        }
    }

    #[test]
    fn test_unescape() {
        let tests = [
            ("bar~0", Some("bar~")),
            ("bar~1", Some("bar/")),
            ("bar~01", Some("bar~1")),
            ("bar~", None),
            ("bar~~", None),
        ];
        for (tok, want) in tests {
            let res = JsonPointer::unescape(tok).ok();
            let got = res.as_ref().map(|c| c.as_ref());
            assert_eq!(got, want, "unescape({:?})", tok)
        }
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

    #[test]
    fn test_hashed_value() {
        let mut seen = AHashMap::with_capacity(10);
        let (v1, v2) = (json!(2), json!(2.0));
        assert!(equals(&v1, &v2));
        assert!(seen.insert(HashedValue(&v1), 1).is_none());
        assert!(seen.insert(HashedValue(&v2), 1).is_some());
    }
}
