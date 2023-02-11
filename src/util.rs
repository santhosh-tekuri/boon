use std::{borrow::Cow, fmt::Display, str::Utf8Error};

use percent_encoding::percent_decode_str;
use url::Url;

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
    let mut url = Url::parse("http://a.com").unwrap();
    url.path_segments_mut().unwrap().push(s);
    let s = url.as_str();
    s[s.rfind('/').unwrap() + 1..].to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote() {
        assert_eq!(quote(r#"abc"def'ghi"#), r#"'abc"def\'ghi'"#);
    }

    #[test]
    fn test_path_escape() {
        assert_eq!(
            path_escape("my/cool+blog&about,stuff"),
            "my%2Fcool+blog&about,stuff",
        );
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
}
