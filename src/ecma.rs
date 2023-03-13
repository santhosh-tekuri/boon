use std::borrow::Cow;

use regex_syntax::ast::parse::Parser;
use regex_syntax::ast::Error;
use regex_syntax::ast::ErrorKind;

// covert ecma regex to rust regex if possible
// see https://262.ecma-international.org/8.0/#sec-regular-expressions-patterns
pub(crate) fn convert(pattern: &str) -> Cow<str> {
    let mut pattern = Cow::Borrowed(pattern);

    loop {
        let Err(e) = Parser::new().parse(pattern.as_ref()) else {
            break;
        };
        if let Some(s) = fix(e) {
            pattern = Cow::Owned(s);
        } else {
            break;
        }
    }
    pattern
}

fn fix(e: Error) -> Option<String> {
    if let ErrorKind::EscapeUnrecognized = e.kind() {
        let (start, end) = (e.span().start.offset, e.span().end.offset);
        let s = &e.pattern()[start..end];
        match s {
            r#"\/"# => {
                // handle escaping '/'
                return Some(format!("{}/{}", &e.pattern()[..start], &e.pattern()[end..],));
            }
            r#"\c"# => {
                // handle \c{control_letter}
                if let Some(control_letter) = e.pattern()[end..].chars().next() {
                    if control_letter.is_ascii_alphabetic() {
                        return Some(format!(
                            "{}{}{}",
                            &e.pattern()[..start],
                            ((control_letter as u8) % 32) as char,
                            &e.pattern()[end + 1..],
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecma_compat() {
        assert_eq!(convert(r#"ab\/cde\/fg"#), r#"ab/cde/fg"#); // '/' can be escaped
        assert_eq!(convert(r#"ab\cAcde\cBfg"#), "ab\u{1}cde\u{2}fg"); // \c{control_letter}
        assert_eq!(convert(r#"\c\n"#), r#"\c\n"#); // \c{invalid_char}
    }
}
