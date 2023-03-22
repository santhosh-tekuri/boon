use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
};

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::{
    util::*, validator::AbsoluteKeywordLocation, ErrorKind, InstanceLocation, ValidationError,
};

impl<'s, 'v> ValidationError<'s, 'v> {
    pub(crate) fn display(
        &self,
        f: &mut Formatter,
        parent_abs: &str,
        indent: usize,
    ) -> std::fmt::Result {
        let tmp;
        let mut cur_abs;
        if let ErrorKind::Schema { url } = &self.kind {
            debug_assert_eq!(indent, 0, "ErrorKind::Schema must have zero indent");
            write!(f, "jsonschema {}", self.kind)?;
            cur_abs = *url;
        } else {
            tmp = self.absolute_keyword_location.to_string();
            cur_abs = &tmp;

            if let ErrorKind::Reference { .. } = &self.kind {
                if self.causes.len() == 1 {
                    return self.causes[0].display(f, parent_abs, indent);
                }
            }

            // indent --
            if indent > 0 {
                for _ in 0..indent - 1 {
                    write!(f, "  ")?;
                }
                write!(f, "- ")?;
            }

            // location --
            let inst = &self.instance_location;
            write!(f, "at {}", quote(&inst.to_string()))?;
            if f.alternate() {
                if let ErrorKind::Reference { url } = &self.kind {
                    cur_abs = *url;
                }
                let (p, _) = split(parent_abs);
                let (c, frag) = split(cur_abs);
                if c == p {
                    write!(f, " [S#{frag}]")?;
                } else {
                    write!(f, " [{cur_abs}]")?;
                }
            }

            // message --
            if let ErrorKind::Reference { .. } = &self.kind {
                write!(f, "validation failed")?;
            } else {
                write!(f, ": {}", self.kind)?;
            }
        }

        // causes --
        if !self.causes.is_empty() {
            writeln!(f)?;
        }
        for (i, cause) in self.causes.iter().enumerate() {
            if i != 0 {
                writeln!(f)?;
            };
            cause.display(f, cur_abs, indent + 1)?;
        }
        Ok(())
    }

    pub fn flag_output(&self) -> FlagOutput {
        FlagOutput { valid: false }
    }

    pub fn basic_output(&self) -> OutputUnit {
        fn flatten<'e, 's, 'v>(
            err: &'e ValidationError<'s, 'v>,
            kw_loc: &mut String,
            parent_abs: &str,
            mut in_ref: bool,
            tgt: &mut Vec<OutputUnit<'e, 's, 'v>>,
        ) {
            let tmp = err.absolute_keyword_location.to_string();
            let removed = update_keyword_location(kw_loc, parent_abs, &tmp, err);
            let mut cur_abs = tmp.as_str();
            let mut is_ref = false;
            if let ErrorKind::Reference { url } = &err.kind {
                is_ref = true;
                in_ref = true;
                cur_abs = *url;
            }
            if !(is_ref && err.causes.len() == 1) {
                let absolute_keyword_location = if in_ref {
                    if let ErrorKind::Reference { url } = &err.kind {
                        Some(Cow::Owned(AbsoluteKeywordLocation {
                            schema_url: url,
                            keyword_path: None,
                        }))
                    } else {
                        Some(Cow::Borrowed(&err.absolute_keyword_location))
                    }
                } else {
                    None
                };
                tgt.push(OutputUnit {
                    valid: false,
                    keyword_location: kw_loc.to_string(),
                    absolute_keyword_location,
                    instance_location: &err.instance_location,
                    error: OutputError::Leaf(&err.kind),
                });
            }
            let len = kw_loc.len();
            for cause in &err.causes {
                flatten(cause, kw_loc, cur_abs, in_ref, tgt);
                kw_loc.truncate(len);
            }
            kw_loc.push_str(&removed);
        }

        let error = if self.causes.is_empty() {
            OutputError::Leaf(&self.kind)
        } else {
            let abs_url = self.absolute_keyword_location.to_string();
            let mut v = vec![];
            let mut kw_loc = String::new();
            for cause in &self.causes {
                flatten(cause, &mut kw_loc, &abs_url, false, &mut v);
                kw_loc.truncate(0);
            }
            OutputError::Branch(v)
        };
        OutputUnit {
            valid: false,
            keyword_location: String::new(),
            absolute_keyword_location: None,
            instance_location: &self.instance_location,
            error,
        }
    }

    pub fn detailed_output(&self) -> OutputUnit {
        fn output_unit<'e, 's, 'v>(
            err: &'e ValidationError<'s, 'v>,
            kw_loc: &mut String,
            parent_abs: &str,
            mut in_ref: bool,
        ) -> OutputUnit<'e, 's, 'v> {
            let temp = err.absolute_keyword_location.to_string();
            let removed = update_keyword_location(kw_loc, parent_abs, &temp, err);
            let mut cur_abs = temp.as_str();
            let mut is_ref = false;
            if let ErrorKind::Reference { url } = &err.kind {
                is_ref = true;
                in_ref = true;
                cur_abs = *url;
            }
            if is_ref && err.causes.len() == 1 {
                let len = kw_loc.len();
                let out = output_unit(&err.causes[0], kw_loc, cur_abs, in_ref);
                kw_loc.truncate(len);
                kw_loc.push_str(&removed);
                return out;
            }

            let absolute_keyword_location = if in_ref {
                if let ErrorKind::Reference { url } = &err.kind {
                    Some(Cow::Owned(AbsoluteKeywordLocation {
                        schema_url: url,
                        keyword_path: None,
                    }))
                } else {
                    Some(Cow::Borrowed(&err.absolute_keyword_location))
                }
            } else {
                None
            };

            let error = if err.causes.is_empty() {
                OutputError::Leaf(&err.kind)
            } else {
                let mut v = vec![];
                let len = kw_loc.len();
                for cause in &err.causes {
                    v.push(output_unit(cause, kw_loc, cur_abs, in_ref));
                    kw_loc.truncate(len);
                }
                OutputError::Branch(v)
            };

            let out = OutputUnit {
                valid: false,
                keyword_location: kw_loc.to_string(),
                absolute_keyword_location,
                instance_location: &err.instance_location,
                error,
            };
            kw_loc.push_str(&removed);
            out
        }
        output_unit(self, &mut String::new(), "", false)
    }
}

pub struct FlagOutput {
    pub valid: bool,
}

impl Serialize for FlagOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("valid", &self.valid)?;
        map.end()
    }
}

impl Display for FlagOutput {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

pub struct OutputUnit<'e, 's, 'v> {
    pub valid: bool,
    pub keyword_location: String,
    pub absolute_keyword_location: Option<Cow<'e, AbsoluteKeywordLocation<'s>>>,
    pub instance_location: &'e InstanceLocation<'v>,
    pub error: OutputError<'e, 's, 'v>,
}

impl<'e, 's, 'v> Serialize for OutputUnit<'e, 's, 'v> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = 4 + self.absolute_keyword_location.as_ref().map_or(0, |_| 1);
        let mut map = serializer.serialize_map(Some(n))?;
        map.serialize_entry("valid", &self.valid)?;
        map.serialize_entry("keywordLocation", &self.keyword_location.to_string())?;
        if let Some(s) = &self.absolute_keyword_location {
            map.serialize_entry("absoluteKeywordLocation", &s.to_string())?;
        }
        map.serialize_entry("instanceLocation", &self.instance_location.to_string())?;
        let pname = match self.error {
            OutputError::Leaf(_) => "error",
            OutputError::Branch(_) => "errors",
        };
        map.serialize_entry(pname, &self.error)?;
        map.end()
    }
}

impl<'e, 's, 'v> Display for OutputUnit<'e, 's, 'v> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

pub enum OutputError<'e, 's, 'v> {
    Leaf(&'e ErrorKind<'s>),
    Branch(Vec<OutputUnit<'e, 's, 'v>>),
}

impl<'e, 's, 'v> Serialize for OutputError<'e, 's, 'v> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OutputError::Leaf(kind) => serializer.serialize_str(&kind.to_string()),
            OutputError::Branch(units) => {
                let mut seq = serializer.serialize_seq(Some(units.len()))?;
                for unit in units {
                    seq.serialize_element(unit)?;
                }
                seq.end()
            }
        }
    }
}

fn update_keyword_location(
    kw_loc: &mut String,
    mut parent: &str,
    current: &str,
    err: &ValidationError,
) -> String {
    if parent.is_empty() {
        return String::new();
    }
    if let ErrorKind::Reference { .. } = &err.kind {
        let Some(kw_path) = &err.absolute_keyword_location.keyword_path else {
            debug_assert!(false, "ErrorKind::Reference must has KeywordPath");
            return String::new();
        };
        kw_loc.push('/');
        kw_loc.push_str(kw_path.keyword);
        String::new()
    } else {
        let mut removed = String::new();

        // handle minContains with child contains
        while !current.starts_with(parent) {
            let Some(slash) = parent.rfind('/') else {
                debug_assert!(false, "ErrorKind::Reference must has KeywordPath");
                return String::new();
            };
            removed.insert_str(0, &parent[slash..]);
            parent = &parent[..slash];
        }

        let kw_path = &current[parent.len()..]; // todo: url-decode
        kw_loc.push_str(kw_path);
        removed
    }
}
