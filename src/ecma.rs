use std::borrow::Cow;

use regex_syntax::ast::parse::Parser;
use regex_syntax::ast::{self, *};

// covert ecma regex to rust regex if possible
// see https://262.ecma-international.org/11.0/#sec-regexp-regular-expression-objects
pub(crate) fn convert(pattern: &str) -> Result<Cow<str>, Box<dyn std::error::Error>> {
    let mut pattern = Cow::Borrowed(pattern);

    let mut ast = loop {
        match Parser::new().parse(pattern.as_ref()) {
            Ok(ast) => break ast,
            Err(e) => {
                if let Some(s) = fix_error(&e) {
                    pattern = Cow::Owned(s);
                } else {
                    Err(e)?;
                }
            }
        }
    };

    loop {
        let translator = Translator {
            pat: pattern.as_ref(),
            out: None,
        };
        if let Some(updated_pattern) = ast::visit(&ast, translator)? {
            match Parser::new().parse(&updated_pattern) {
                Ok(updated_ast) => {
                    pattern = Cow::Owned(updated_pattern);
                    ast = updated_ast;
                }
                Err(e) => {
                    debug_assert!(
                        false,
                        "ecma::translate changed {:?} to {:?}: {e}",
                        pattern, updated_pattern
                    );
                    break;
                }
            }
        } else {
            break;
        }
    }
    Ok(pattern)
}

fn fix_error(e: &Error) -> Option<String> {
    if let ErrorKind::EscapeUnrecognized = e.kind() {
        let (start, end) = (e.span().start.offset, e.span().end.offset);
        let s = &e.pattern()[start..end];
        if let r"\c" = s {
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
    }
    None
}

/**
handles following translations:
-  \d should ascii digits only. so replace with [0-9]
-  \D should match everything but ascii digits. so replace with [^0-9]
-  \w should match ascii letters only. so replace with [a-zA-Z0-9_]
-  \W should match everything but ascii letters. so replace with [^a-zA-Z0-9_]
-  \s and \S differences
-  \a is not an ECMA 262 control escape
*/
struct Translator<'a> {
    pat: &'a str,
    out: Option<String>,
}

impl Translator<'_> {
    fn replace(&mut self, span: &Span, with: &str) {
        let (start, end) = (span.start.offset, span.end.offset);
        self.out = Some(format!("{}{with}{}", &self.pat[..start], &self.pat[end..]));
    }

    fn replace_class_class(&mut self, perl: &ClassPerl) {
        match perl.kind {
            ClassPerlKind::Digit => {
                self.replace(&perl.span, if perl.negated { "[^0-9]" } else { "[0-9]" });
            }
            ClassPerlKind::Word => {
                let with = &if perl.negated {
                    "[^A-Za-z0-9_]"
                } else {
                    "[A-Za-z0-9_]"
                };
                self.replace(&perl.span, with);
            }
            ClassPerlKind::Space => {
                let with = &if perl.negated {
                    "[^ \t\n\r\u{000b}\u{000c}\u{00a0}\u{feff}\u{2003}\u{2029}]"
                } else {
                    "[ \t\n\r\u{000b}\u{000c}\u{00a0}\u{feff}\u{2003}\u{2029}]"
                };
                self.replace(&perl.span, with);
            }
        }
    }
}

impl Visitor for Translator<'_> {
    type Output = Option<String>;
    type Err = &'static str;

    fn finish(self) -> Result<Self::Output, Self::Err> {
        Ok(self.out)
    }

    fn visit_class_set_item_pre(&mut self, ast: &ast::ClassSetItem) -> Result<(), Self::Err> {
        if let ClassSetItem::Perl(perl) = ast {
            self.replace_class_class(perl);
        }
        Ok(())
    }

    fn visit_post(&mut self, ast: &Ast) -> Result<(), Self::Err> {
        if self.out.is_some() {
            return Ok(());
        }
        match ast {
            Ast::ClassPerl(perl) => {
                self.replace_class_class(perl);
            }
            Ast::Literal(ref literal) => {
                if let Literal {
                    kind: LiteralKind::Special(SpecialLiteralKind::Bell),
                    ..
                } = literal.as_ref()
                {
                    return Err("\\a is not an ECMA 262 control escape");
                }
            }
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecma_compat_valid() {
        // println!("{:#?}", Parser::new().parse(r#"a\a"#));
        let tests = [
            (r"ab\cAcde\cBfg", "ab\u{1}cde\u{2}fg"), // \c{control_letter}
            (r"\\comment", r"\\comment"),            // there is no \c
            (r"ab\def", r#"ab[0-9]ef"#),             // \d
            (r"ab[a-z\d]ef", r#"ab[a-z[0-9]]ef"#),   // \d inside classSet
            (r"ab\Def", r#"ab[^0-9]ef"#),            // \d
            (r"ab[a-z\D]ef", r#"ab[a-z[^0-9]]ef"#),  // \D inside classSet
        ];
        for (input, want) in tests {
            match convert(input) {
                Ok(got) => {
                    if got.as_ref() != want {
                        panic!("convert({input:?}): got: {got:?}, want: {want:?}");
                    }
                }
                Err(e) => {
                    panic!("convert({input:?}) failed: {e}");
                }
            }
        }
    }

    #[test]
    fn test_ecma_compat_invalid() {
        // println!("{:#?}", Parser::new().parse(r#"a\a"#));
        let tests = [
            r"\c\n",     // \c{invalid_char}
            r"abc\adef", // \a is not valid
        ];
        for input in tests {
            if convert(input).is_ok() {
                panic!("convert({input:?}) mut fail");
            }
        }
    }
}
