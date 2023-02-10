#![allow(dead_code)]

mod compiler;
mod draft;
mod loader;
mod root;
mod roots;
mod util;

use std::{borrow::Cow, collections::HashMap};

use regex::Regex;
use serde_json::{Number, Value};

struct Schema {
    types: Vec<Type>,
    enum_: Vec<Value>,
    constant: Option<Value>,

    min_properties: Option<usize>,
    max_properties: Option<usize>,
    required: Vec<String>,
    dependent_required: HashMap<String, Vec<String>>,

    min_items: Option<usize>,
    max_items: Option<usize>,
    unique_items: bool,

    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Regex>,
    content_encoding: Option<String>,
    decoder: Option<Decoder>,
    content_media_type: Option<String>,
    media_type: Option<MediaType>,

    minimum: Option<Number>,
    maximum: Option<Number>,
    exclusive_minimum: Option<Number>,
    exclusive_maximum: Option<Number>,
    multiple_of: Option<Number>,
}

impl Schema {
    fn validate(&self, v: &Value) -> Result<(), ErrorKind> {
        if !self.types.is_empty() {
            let v_type = Type::of(v);
            let matched = self.types.iter().any(|t| {
                if *t == Type::Integer && v_type == Type::Number {
                    if let Value::Number(n) = v {
                        return n.is_i64() || n.is_u64();
                    }
                }
                *t == v_type
            });
            if !matched {
                return Err(ErrorKind::Type {
                    got: v_type,
                    want: self.types.clone(),
                });
            }
        }

        if !self.enum_.is_empty() && !self.enum_.contains(v) {
            return Err(ErrorKind::Enum {
                got: v.clone(),
                want: self.enum_.clone(),
            });
        }

        if let Some(c) = &self.constant {
            if v != c {
                return Err(ErrorKind::Const {
                    got: v.clone(),
                    want: c.clone(),
                });
            }
        }

        match v {
            Value::Object(obj) => {
                if let Some(min) = self.min_properties {
                    if obj.len() < min {
                        return Err(ErrorKind::MinProperties {
                            got: obj.len(),
                            want: min,
                        });
                    }
                }
                if let Some(max) = self.max_properties {
                    if obj.len() > max {
                        return Err(ErrorKind::MaxProperties {
                            got: obj.len(),
                            want: max,
                        });
                    }
                }
                let missing = self
                    .required
                    .iter()
                    .filter(|p| !obj.contains_key(p.as_str()))
                    .cloned()
                    .collect::<Vec<String>>();
                if !missing.is_empty() {
                    return Err(ErrorKind::Required { want: missing });
                }

                for (pname, required) in &self.dependent_required {
                    if obj.contains_key(pname) {
                        let missing = required
                            .iter()
                            .filter(|p| !obj.contains_key(p.as_str()))
                            .cloned()
                            .collect::<Vec<String>>();
                        if !missing.is_empty() {
                            return Err(ErrorKind::DependentRequired {
                                got: pname.clone(),
                                want: missing,
                            });
                        }
                    }
                }
            }
            Value::Array(arr) => {
                if let Some(min) = self.min_items {
                    if arr.len() < min {
                        return Err(ErrorKind::MinItems {
                            got: arr.len(),
                            want: min,
                        });
                    }
                }
                if let Some(max) = self.max_items {
                    if arr.len() > max {
                        return Err(ErrorKind::MaxItems {
                            got: arr.len(),
                            want: max,
                        });
                    }
                }
                if self.unique_items {
                    for i in 1..arr.len() {
                        for j in 0..i {
                            if arr[i] == arr[j] {
                                return Err(ErrorKind::UniqueItems { got: [i, j] });
                            }
                        }
                    }
                }
            }
            Value::String(s) => {
                let mut len = None;
                if let Some(min) = self.min_length {
                    let len = len.get_or_insert_with(|| s.chars().count());
                    if *len < min {
                        return Err(ErrorKind::MinLength {
                            got: *len,
                            want: min,
                        });
                    }
                }
                if let Some(max) = self.max_length {
                    let len = len.get_or_insert_with(|| s.chars().count());
                    if *len > max {
                        return Err(ErrorKind::MaxLength {
                            got: *len,
                            want: max,
                        });
                    }
                }
                if let Some(regex) = &self.pattern {
                    if !regex.is_match(s) {
                        return Err(ErrorKind::Pattern {
                            got: s.clone(),
                            want: regex.as_str().to_string(),
                        });
                    }
                }

                let mut decoded = Cow::from(s.as_bytes());
                if let Some(decode) = &self.decoder {
                    match decode(s) {
                        Some(bytes) => decoded = Cow::from(bytes),
                        None => {
                            return Err(ErrorKind::ContentEncoding {
                                got: s.clone(),
                                want: self.content_encoding.clone().unwrap(),
                            })
                        }
                    }
                }
                if let Some(media_type) = &self.media_type {
                    if !media_type(decoded.as_ref()) {
                        return Err(ErrorKind::ContentMediaType {
                            got: decoded.into_owned(),
                            want: self.content_media_type.clone().unwrap(),
                        });
                    }
                }
            }
            Value::Number(n) => {
                if let Some(min) = &self.minimum {
                    if let (Some(minf), Some(vf)) = (min.as_f64(), n.as_f64()) {
                        if vf < minf {
                            return Err(ErrorKind::Minimum {
                                got: n.clone(),
                                want: min.clone(),
                            });
                        }
                    }
                }
                if let Some(max) = &self.maximum {
                    if let (Some(maxf), Some(vf)) = (max.as_f64(), n.as_f64()) {
                        if vf > maxf {
                            return Err(ErrorKind::Maximum {
                                got: n.clone(),
                                want: max.clone(),
                            });
                        }
                    }
                }
                if let Some(ex_min) = &self.exclusive_minimum {
                    if let (Some(ex_minf), Some(nf)) = (ex_min.as_f64(), n.as_f64()) {
                        if nf <= ex_minf {
                            return Err(ErrorKind::ExclusiveMinimum {
                                got: n.clone(),
                                want: ex_min.clone(),
                            });
                        }
                    }
                }
                if let Some(ex_max) = &self.exclusive_maximum {
                    if let (Some(ex_maxf), Some(nf)) = (ex_max.as_f64(), n.as_f64()) {
                        if nf >= ex_maxf {
                            return Err(ErrorKind::ExclusiveMaximum {
                                got: n.clone(),
                                want: ex_max.clone(),
                            });
                        }
                    }
                }
                if let Some(mul) = &self.multiple_of {
                    if let (Some(mulf), Some(nf)) = (mul.as_f64(), n.as_f64()) {
                        if (nf / mulf).fract() != 0.0 {
                            return Err(ErrorKind::MultipleOf {
                                got: n.clone(),
                                want: mul.clone(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone)]
enum Type {
    Null,
    Bool,
    Number,
    Integer,
    String,
    Array,
    Object,
}

impl Type {
    fn of(v: &Value) -> Self {
        match v {
            Value::Null => Type::Null,
            Value::Bool(_) => Type::Bool,
            Value::Number(_) => Type::Number,
            Value::String(_) => Type::String,
            Value::Array(_) => Type::Array,
            Value::Object(_) => Type::Object,
        }
    }
}

enum ErrorKind {
    Type { got: Type, want: Vec<Type> },
    Enum { got: Value, want: Vec<Value> },
    Const { got: Value, want: Value },
    MinProperties { got: usize, want: usize },
    MaxProperties { got: usize, want: usize },
    Required { want: Vec<String> },
    DependentRequired { got: String, want: Vec<String> },
    MinItems { got: usize, want: usize },
    MaxItems { got: usize, want: usize },
    UniqueItems { got: [usize; 2] },
    MinLength { got: usize, want: usize },
    MaxLength { got: usize, want: usize },
    Pattern { got: String, want: String },
    ContentEncoding { got: String, want: String },
    ContentMediaType { got: Vec<u8>, want: String },
    Minimum { got: Number, want: Number },
    Maximum { got: Number, want: Number },
    ExclusiveMinimum { got: Number, want: Number },
    ExclusiveMaximum { got: Number, want: Number },
    MultipleOf { got: Number, want: Number },
}

type Decoder = Box<dyn Fn(&str) -> Option<Vec<u8>>>;
type MediaType = Box<dyn Fn(&[u8]) -> bool>;
