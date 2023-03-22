use std::fmt::{Display, Formatter};

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::{
    util::*, validator::AbsoluteKeywordLocation, ErrorKind, InstanceLocation, ValidationError,
};

// todo: remove ErrorKind::Reference Usage

impl<'s, 'v> ValidationError<'s, 'v> {
    pub(crate) fn display(&self, f: &mut Formatter, indent: usize) -> std::fmt::Result {
        if let ErrorKind::Schema { .. } = &self.kind {
            debug_assert_eq!(indent, 0, "ErrorKind::Schema must have zero indent");
            write!(f, "jsonschema {}", self.kind)?;
        } else {
            let abs_url = self.absolute_keyword_location.to_string();
            let (s, frag) = split(&abs_url);

            if let ErrorKind::Reference { url } = &self.kind {
                if !f.alternate() {
                    return self.causes[0].display(f, indent);
                } else if self.causes.len() == 1 {
                    let (u, _) = split(url);
                    if u == s {
                        return self.causes[0].display(f, indent);
                    }
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
                write!(f, " [S#{frag}] [{abs_url}]")?;
            }

            // message --
            if let ErrorKind::Reference { url } = &self.kind {
                write!(f, "=> [{url}]")?;
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
            cause.display(f, indent + 1)?;
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
            mut in_ref: bool,
            tgt: &mut Vec<OutputUnit<'e, 's, 'v>>,
        ) {
            if let Some(kw_path) = &err.absolute_keyword_location.keyword_path {
                kw_loc.push('/');
                use std::fmt::Write;
                write!(kw_loc, "{kw_path}").expect("write! to string should not fail");
            }

            in_ref = in_ref || matches!(err.kind, ErrorKind::Reference { .. });
            let absolute_keyword_location = if in_ref {
                Some(&err.absolute_keyword_location)
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
            let len = kw_loc.len();
            for cause in &err.causes {
                flatten(cause, kw_loc, in_ref, tgt);
                kw_loc.truncate(len);
            }
        }
        let error = if self.causes.is_empty() {
            OutputError::Leaf(&self.kind)
        } else {
            let mut v = vec![];
            let mut kw_loc = String::new();
            for cause in &self.causes {
                flatten(cause, &mut kw_loc, false, &mut v);
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
            mut in_ref: bool,
        ) -> OutputUnit<'e, 's, 'v> {
            if let Some(kw_path) = &err.absolute_keyword_location.keyword_path {
                kw_loc.push('/');
                use std::fmt::Write;
                write!(kw_loc, "{kw_path}").expect("write! to string should not fail");
            }

            in_ref = in_ref || matches!(err.kind, ErrorKind::Reference { .. });
            let absolute_keyword_location = if in_ref {
                Some(&err.absolute_keyword_location)
            } else {
                None
            };

            let error = if err.causes.is_empty() {
                OutputError::Leaf(&err.kind)
            } else {
                let mut v = vec![];
                let len = kw_loc.len();
                for cause in &err.causes {
                    v.push(output_unit(cause, kw_loc, in_ref));
                    kw_loc.truncate(len);
                }
                OutputError::Branch(v)
            };

            OutputUnit {
                valid: false,
                keyword_location: kw_loc.to_string(),
                absolute_keyword_location,
                instance_location: &err.instance_location,
                error,
            }
        }
        output_unit(self, &mut String::new(), false)
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
    pub absolute_keyword_location: Option<&'e AbsoluteKeywordLocation<'s>>,
    pub instance_location: &'e InstanceLocation<'v>,
    pub error: OutputError<'e, 's, 'v>,
}

impl<'e, 's, 'v> Serialize for OutputUnit<'e, 's, 'v> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = 4 + self.absolute_keyword_location.map_or(0, |_| 1);
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
