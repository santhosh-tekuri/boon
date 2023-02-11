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
use util::escape;

struct SchemaIndex(usize);

#[derive(Default)]
struct Schemas {
    list: Vec<Schema>,
    map: HashMap<String, usize>,
}

impl Schemas {
    fn enqueue(&self, queue: &mut Vec<String>, loc: String) -> usize {
        queue.push(loc);
        self.list.len() + queue.len() - 1
    }

    fn insert(&mut self, loc: String, sch: Schema) -> SchemaIndex {
        self.list.push(sch);
        let index = self.list.len() - 1;
        self.map.insert(loc, index);
        SchemaIndex(index)
    }

    fn get(&self, index: SchemaIndex) -> Option<&Schema> {
        self.list.get(index.0)
    }

    fn get_by_loc(&self, loc: &str) -> Option<&Schema> {
        self.map.get(loc).and_then(|&i| self.list.get(i))
    }
}

macro_rules! error {
    ($kw_path:expr, $inst_path:expr, $kind:ident, $name:ident: $value:expr) => {
        Err(ValidationError {
            absolute_keyword_location: $kw_path.into(),
            instance_location: $inst_path.clone(),
            kind: ErrorKind::$kind { $name: $value },
        })
    };
    ($kw_path:expr, $inst_path:expr, $kind:ident, $got:expr, $want:expr) => {
        Err(ValidationError {
            absolute_keyword_location: $kw_path.into(),
            instance_location: $inst_path.clone(),
            kind: ErrorKind::$kind {
                got: $got,
                want: $want,
            },
        })
    };
}

#[derive(Default)]
struct Schema {
    loc: String,
    vocab: Vec<String>,

    // type agnostic --
    types: Vec<Type>,
    enum_: Vec<Value>,
    constant: Option<Value>,
    not: Option<usize>,
    all_of: Vec<usize>,
    any_of: Vec<usize>,
    one_of: Vec<usize>,
    if_: Option<usize>,
    then: Option<usize>,
    else_: Option<usize>,

    // object --
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    required: Vec<String>,
    properties: HashMap<String, usize>,
    property_names: Option<usize>,
    additional_properties: Option<AdditionalProperties>,
    dependent_required: HashMap<String, Vec<String>>,
    dependent_schemas: HashMap<String, usize>,
    dependencies: HashMap<String, Dependency>,

    // array --
    min_items: Option<usize>,
    max_items: Option<usize>,
    unique_items: bool,
    min_contains: Option<usize>,
    max_contains: Option<usize>,
    contains: Option<usize>,
    prefix_items: Option<usize>,
    items: Option<Items>,
    items2020: Option<usize>,

    // string --
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Regex>,
    content_encoding: Option<String>,
    decoder: Option<Decoder>,
    content_media_type: Option<String>,
    media_type: Option<MediaType>,

    // number --
    minimum: Option<Number>,
    maximum: Option<Number>,
    exclusive_minimum: Option<Number>,
    exclusive_maximum: Option<Number>,
    multiple_of: Option<Number>,
}

//#[derive(Debug)]
enum Items {
    SchemaRef(usize),
    SchemaRefs(Vec<usize>),
}

//#[derive(Debug)]
enum AdditionalProperties {
    Bool(bool),
    SchemaRef(usize),
}

//#[derive(Debug)]
enum Dependency {
    Props(Vec<String>),
    SchemaRef(usize),
}

impl Schema {
    fn new(loc: String) -> Self {
        Self {
            loc,
            ..Default::default()
        }
    }

    fn has_vocab(&self, _name: &str) -> bool {
        todo!();
    }

    pub(crate) fn validate(&self, v: &Value, vloc: String) -> Result<(), ValidationError> {
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
                return error!("type", vloc, Type, v_type, self.types.clone());
            }
        }

        if !self.enum_.is_empty() && !self.enum_.contains(v) {
            return error!("enum", vloc, Enum, v.clone(), self.enum_.clone());
        }

        if let Some(c) = &self.constant {
            if v != c {
                return error!("const", vloc, Const, v.clone(), c.clone());
            }
        }

        match v {
            Value::Object(obj) => {
                if let Some(min) = self.min_properties {
                    if obj.len() < min {
                        return error!("minProperties", vloc, MinProperties, obj.len(), min);
                    }
                }
                if let Some(max) = self.max_properties {
                    if obj.len() > max {
                        return error!("maxProperties", vloc, MaxProperties, obj.len(), max);
                    }
                }
                let missing = self
                    .required
                    .iter()
                    .filter(|p| !obj.contains_key(p.as_str()))
                    .cloned()
                    .collect::<Vec<String>>();
                if !missing.is_empty() {
                    return error!("required", vloc, Required, want: missing);
                }

                for (pname, required) in &self.dependent_required {
                    if obj.contains_key(pname) {
                        let missing = required
                            .iter()
                            .filter(|p| !obj.contains_key(p.as_str()))
                            .cloned()
                            .collect::<Vec<String>>();
                        if !missing.is_empty() {
                            return error!(
                                format!("dependentRequired/{}", escape(pname)),
                                vloc,
                                DependentRequired,
                                pname.clone(),
                                missing
                            );
                        }
                    }
                }
            }
            Value::Array(arr) => {
                if let Some(min) = self.min_items {
                    if arr.len() < min {
                        return error!("minItems", vloc, MinItems, arr.len(), min);
                    }
                }
                if let Some(max) = self.max_items {
                    if arr.len() > max {
                        return error!("maxItems", vloc, MaxItems, arr.len(), max);
                    }
                }
                if self.unique_items {
                    for i in 1..arr.len() {
                        for j in 0..i {
                            if arr[i] == arr[j] {
                                return error!("uniqueItems", vloc, UniqueItems, got: [i, j]);
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
                        return error!("minLength", vloc, MinLength, *len, min);
                    }
                }
                if let Some(max) = self.max_length {
                    let len = len.get_or_insert_with(|| s.chars().count());
                    if *len > max {
                        return error!("maxLength", vloc, MaxLength, *len, max);
                    }
                }
                if let Some(regex) = &self.pattern {
                    if !regex.is_match(s) {
                        return error!(
                            "pattern",
                            vloc,
                            Pattern,
                            s.clone(),
                            regex.as_str().to_string()
                        );
                    }
                }

                let mut decoded = Cow::from(s.as_bytes());
                if let Some(decode) = &self.decoder {
                    match decode(s) {
                        Some(bytes) => decoded = Cow::from(bytes),
                        None => {
                            return error!(
                                "contentEncoding",
                                vloc,
                                ContentEncoding,
                                s.clone(),
                                self.content_encoding.clone().unwrap()
                            )
                        }
                    }
                }
                if let Some(media_type) = &self.media_type {
                    if !media_type(decoded.as_ref()) {
                        return error!(
                            "contentMediaType",
                            vloc,
                            ContentMediaType,
                            decoded.into_owned(),
                            self.content_media_type.clone().unwrap()
                        );
                    }
                }
            }
            Value::Number(n) => {
                if let Some(min) = &self.minimum {
                    if let (Some(minf), Some(vf)) = (min.as_f64(), n.as_f64()) {
                        if vf < minf {
                            return error!("minimum", vloc, Minimum, n.clone(), min.clone());
                        }
                    }
                }
                if let Some(max) = &self.maximum {
                    if let (Some(maxf), Some(vf)) = (max.as_f64(), n.as_f64()) {
                        if vf > maxf {
                            return error!("maximum", vloc, Maximum, n.clone(), max.clone());
                        }
                    }
                }
                if let Some(ex_min) = &self.exclusive_minimum {
                    if let (Some(ex_minf), Some(nf)) = (ex_min.as_f64(), n.as_f64()) {
                        if nf <= ex_minf {
                            return error!(
                                "exclusiveMinimum",
                                vloc,
                                ExclusiveMinimum,
                                n.clone(),
                                ex_min.clone()
                            );
                        }
                    }
                }
                if let Some(ex_max) = &self.exclusive_maximum {
                    if let (Some(ex_maxf), Some(nf)) = (ex_max.as_f64(), n.as_f64()) {
                        if nf >= ex_maxf {
                            return error!(
                                "exclusiveMaximum",
                                vloc,
                                ExclusiveMaximum,
                                n.clone(),
                                ex_max.clone()
                            );
                        }
                    }
                }
                if let Some(mul) = &self.multiple_of {
                    if let (Some(mulf), Some(nf)) = (mul.as_f64(), n.as_f64()) {
                        if (nf / mulf).fract() != 0.0 {
                            return error!("multipleOf", vloc, MultipleOf, n.clone(), mul.clone());
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
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "null" => Some(Self::Null),
            "boolean" => Some(Self::Bool),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            "string" => Some(Self::String),
            "array" => Some(Self::Array),
            "object" => Some(Self::Object),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct ValidationError {
    absolute_keyword_location: String,
    instance_location: String,
    kind: ErrorKind,
}

#[derive(Debug)]
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
