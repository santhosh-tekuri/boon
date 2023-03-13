use std::borrow::Cow;

use regex_syntax::ast::parse::Parser;
use regex_syntax::ast::{self, *};

// covert ecma regex to rust regex if possible
// see https://262.ecma-international.org/11.0/#sec-regexp-regular-expression-objects
pub(crate) fn convert(pattern: &str) -> Result<Cow<str>, &'static str> {
    let mut pattern = Cow::Borrowed(pattern);

    let mut ast = loop {
        match Parser::new().parse(pattern.as_ref()) {
            Ok(ast) => break Some(ast),
            Err(e) => {
                if let Some(s) = fix(e) {
                    pattern = Cow::Owned(s);
                } else {
                    break None;
                }
            }
        }
    };

    while let Some(t) = ast {
        let translator = Translator {
            pat: pattern.as_ref(),
            out: None,
        };
        let x = ast::visit(&t, translator)?;
        if let Some(s) = x {
            match Parser::new().parse(&s) {
                Ok(t) => {
                    pattern = Cow::Owned(s);
                    ast = Some(t);
                }
                Err(e) => {
                    debug_assert!(
                        false,
                        "ecma::translate changed {:?} to {:?}: {e}",
                        pattern, s
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
            r#"\D"# => {
                // handle \c{control_letter}
                if let Some(control_letter) = e.pattern()[end..].chars().next() {
                    if control_letter.is_ascii_alphabetic() {
                        return Some(format!(
                            "{}[^0-9]{}",
                            &e.pattern()[..start],
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

impl<'a> Translator<'a> {
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

impl<'a> Visitor for Translator<'a> {
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
            Ast::Class(Class::Perl(perl)) => {
                self.replace_class_class(perl);
            }
            Ast::Literal(Literal {
                kind: LiteralKind::Special(SpecialLiteralKind::Bell),
                ..
            }) => return Err("\\a is not an ECMA 262 control escape"),
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecma_compat() {
        // println!("{:#?}", Parser::new().parse(r#"a\a"#));
        let tests = [
            (r#"ab\/cde\/fg"#, r#"ab/cde/fg"#),        // '/' can be escaped
            (r#"ab\cAcde\cBfg"#, "ab\u{1}cde\u{2}fg"), // \c{control_letter}
            (r#"\c\n"#, r#"\c\n"#),                    // \c{invalid_char}
            (r#"\\comment"#, r#"\\comment"#),          // not \c{invalid_char}
            (r#"ab\def"#, r#"ab[0-9]ef"#),             // \d
            (r#"ab[a-z\d]ef"#, r#"ab[a-z[0-9]]ef"#),   // \d inside classSet
            (r#"ab\Def"#, r#"ab[^0-9]ef"#),            // \d
            (r#"ab[a-z\D]ef"#, r#"ab[a-z[^0-9]]ef"#),  // \D inside classSet
        ];
        for (input, want) in tests {
            let got = convert(input);
            if got != Ok(want.into()) {
                panic!("convert({input:?}): got: {got:?}, want: {want:?}");
            }
        }
    }
}
