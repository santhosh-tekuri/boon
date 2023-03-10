use std::fmt::Display;

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::{util::write_json_to_fmt, ErrorKind, ValidationError};

impl ValidationError {
    pub fn flag_output(&self) -> FlagOutput {
        FlagOutput { valid: false }
    }

    pub fn basic_output(&self) -> OutputUnit {
        fn flatten<'a>(err: &'a ValidationError, mut in_ref: bool, v: &mut Vec<OutputUnit<'a>>) {
            in_ref = in_ref || matches!(&err.kind, ErrorKind::Reference { .. });
            let absolute_keyword_location = if in_ref {
                Some(err.absolute_keyword_location.as_str())
            } else {
                None
            };
            v.push(OutputUnit {
                valid: false,
                keyword_location: &err.keyword_location,
                absolute_keyword_location,
                instance_location: &err.instance_location,
                error: OutputError::Single(&err.kind),
            });
            for cause in &err.causes {
                flatten(cause, in_ref, v);
            }
        }
        let error = if self.causes.is_empty() {
            OutputError::Single(&self.kind)
        } else {
            let mut v = vec![];
            for cause in &self.causes {
                flatten(cause, false, &mut v);
            }
            OutputError::Multiple(v)
        };
        OutputUnit {
            valid: false,
            keyword_location: &self.keyword_location,
            absolute_keyword_location: None,
            instance_location: &self.instance_location,
            error,
        }
    }

    pub fn detailed_output(&self) -> OutputUnit {
        fn output_unit(err: &ValidationError, mut in_ref: bool) -> OutputUnit {
            in_ref = in_ref || matches!(&err.kind, ErrorKind::Reference { .. });
            let error = if err.causes.is_empty() {
                OutputError::Single(&err.kind)
            } else {
                let mut v = vec![];
                for cause in &err.causes {
                    v.push(output_unit(cause, in_ref));
                }
                OutputError::Multiple(v)
            };
            let absolute_keyword_location = if in_ref {
                Some(err.absolute_keyword_location.as_str())
            } else {
                None
            };
            OutputUnit {
                valid: false,
                keyword_location: &err.keyword_location,
                absolute_keyword_location,
                instance_location: &err.instance_location,
                error,
            }
        }
        output_unit(self, false)
    }
}

pub struct FlagOutput {
    valid: bool,
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

pub struct OutputUnit<'a> {
    pub valid: bool,
    pub keyword_location: &'a str,
    pub absolute_keyword_location: Option<&'a str>,
    pub instance_location: &'a str,
    pub error: OutputError<'a>,
}

impl<'a> Serialize for OutputUnit<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = 4 + self.absolute_keyword_location.map_or(0, |_| 1);
        let mut map = serializer.serialize_map(Some(n))?;
        map.serialize_entry("valid", &self.valid)?;
        map.serialize_entry("keywordLocation", &self.keyword_location)?;
        if let Some(s) = &self.absolute_keyword_location {
            map.serialize_entry("absoluteKeywordLocation", s)?;
        }
        map.serialize_entry("instanceLocation", &self.instance_location)?;
        let pname = match self.error {
            OutputError::Single(_) => "error",
            OutputError::Multiple(_) => "errors",
        };
        map.serialize_entry(pname, &self.error)?;
        map.end()
    }
}

impl<'a> Display for OutputUnit<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

pub enum OutputError<'a> {
    Single(&'a ErrorKind),
    Multiple(Vec<OutputUnit<'a>>),
}

impl<'a> Serialize for OutputError<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OutputError::Single(kind) => serializer.serialize_str(&kind.to_string()),
            OutputError::Multiple(units) => {
                let mut seq = serializer.serialize_seq(Some(units.len()))?;
                for unit in units {
                    seq.serialize_element(unit)?;
                }
                seq.end()
            }
        }
    }
}
