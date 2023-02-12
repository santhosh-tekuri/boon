#![allow(dead_code)]

mod compiler;
mod draft;
mod loader;
mod root;
mod roots;
mod util;

pub use compiler::Draft;
pub use compiler::*;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    fmt::Display,
};

use regex::Regex;
use serde_json::{Number, Value};
use util::{equals, escape, join_iter, quote};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaIndex(usize);

#[derive(Default)]
pub struct Schemas {
    list: Vec<Schema>,
    map: HashMap<String, usize>,
}

impl Schemas {
    fn enqueue(&self, queue: &mut VecDeque<String>, loc: String) -> usize {
        if let Some(&index) = self.map.get(&loc) {
            return index;
        }
        if let Some(qindex) = queue.iter().position(|e| *e == loc) {
            return self.list.len() + qindex;
        }
        queue.push_back(loc);
        self.list.len() + queue.len() - 1
    }

    fn insert(&mut self, loc: String, sch: Schema) -> SchemaIndex {
        self.list.push(sch);
        let index = self.list.len() - 1;
        self.map.insert(loc, index);
        SchemaIndex(index)
    }

    fn get(&self, index: usize) -> &Schema {
        &self.list[index] // todo: return bug
    }

    fn get_by_loc(&self, loc: &str) -> Option<&Schema> {
        let mut loc = Cow::from(loc);
        if loc.rfind('#').is_none() {
            let mut s = loc.into_owned();
            s.push('#');
            loc = Cow::from(s);
        }
        self.map.get(loc.as_ref()).and_then(|&i| self.list.get(i))
    }

    /// Validates `v` with schema identified by `sch_index`
    ///
    /// # Panics
    ///
    /// Panics if `sch_index` does not exist. To avoid panic make sure that
    /// `sch_index` is generated for this instance.
    pub fn validate(&self, v: &Value, sch_index: SchemaIndex) -> Result<(), ValidationError> {
        let Some(sch) = self.list.get(sch_index.0) else {
            panic!("Schemas::validate: schema index out of bounds");
        };
        sch.validate(v, String::new(), self).map(|_| ())
    }
}

macro_rules! kind {
    ($kind:ident, $name:ident: $value:expr) => {
        ErrorKind::$kind { $name: $value }
    };
    ($kind:ident, $got:expr, $want:expr) => {
        ErrorKind::$kind {
            got: $got,
            want: $want,
        }
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
    pattern_properties: Vec<(Regex, usize)>,
    property_names: Option<usize>,
    additional_properties: Option<Additional>,
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
    items: Option<Items>,
    additional_items: Option<Additional>,
    prefix_items: Option<usize>,
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

enum Items {
    SchemaRef(usize),
    SchemaRefs(Vec<usize>),
}

enum Additional {
    Bool(bool),
    SchemaRef(usize),
}

enum Dependency {
    Props(Vec<String>),
    SchemaRef(usize),
}

enum Uneval<'v> {
    Props(HashSet<&'v String>),
    Items(HashSet<usize>),
    None,
}

impl<'v> From<&'v Value> for Uneval<'v> {
    fn from(v: &'v Value) -> Self {
        match v {
            Value::Object(obj) => Self::Props(obj.keys().collect()),
            Value::Array(arr) => Self::Items((0..arr.len()).collect()),
            _ => Self::None,
        }
    }
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

    fn validate<'v>(
        &self,
        v: &'v Value,
        vloc: String,
        schemas: &Schemas,
    ) -> Result<Uneval<'v>, ValidationError> {
        let error = |kw_path, kind| {
            Err(ValidationError {
                absolute_keyword_location: format!("{}/{kw_path}", self.loc),
                instance_location: vloc.clone(),
                kind,
            })
        };
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
                return error("type", kind!(Type, v_type, self.types.clone()));
            }
        }

        if !self.enum_.is_empty() && !self.enum_.iter().any(|e| equals(e, v)) {
            return error("enum", kind!(Enum, v.clone(), self.enum_.clone()));
        }

        if let Some(c) = &self.constant {
            if !equals(v, c) {
                return error("const", kind!(Const, v.clone(), c.clone()));
            }
        }

        let mut uneval = Uneval::from(v);
        match v {
            Value::Object(obj) => {
                let Uneval::Props(uneval) = &mut uneval else {
                    unreachable!("object must value Uneval::Props"); 
                };
                if let Some(min) = self.min_properties {
                    if obj.len() < min {
                        return error("minProperties", kind!(MinProperties, obj.len(), min));
                    }
                }
                if let Some(max) = self.max_properties {
                    if obj.len() > max {
                        return error("maxProperties", kind!(MaxProperties, obj.len(), max));
                    }
                }
                let missing = self
                    .required
                    .iter()
                    .filter(|p| !obj.contains_key(p.as_str()))
                    .cloned()
                    .collect::<Vec<String>>();
                if !missing.is_empty() {
                    return error("required", kind!(Required, want: missing));
                }

                for (pname, required) in &self.dependent_required {
                    if obj.contains_key(pname) {
                        let missing = required
                            .iter()
                            .filter(|p| !obj.contains_key(p.as_str()))
                            .cloned()
                            .collect::<Vec<String>>();
                        if !missing.is_empty() {
                            return error(
                                &format!("dependentRequired/{}", escape(pname)),
                                kind!(DependentRequired, pname.clone(), missing),
                            );
                        }
                    }
                }

                for (pname, &psch) in &self.properties {
                    if let Some(pvalue) = obj.get(pname) {
                        uneval.remove(pname);
                        schemas.get(psch).validate(
                            pvalue,
                            format!("{vloc}/{}", escape(pname)),
                            schemas,
                        )?;
                    }
                }

                for (regex, psch) in &self.pattern_properties {
                    for (pname, pvalue) in obj.iter().filter(|(pname, _)| regex.is_match(pname)) {
                        uneval.remove(pname);
                        schemas.get(*psch).validate(
                            pvalue,
                            format!("{vloc}/{}", escape(pname)),
                            schemas,
                        )?;
                    }
                }

                if let Some(additional) = &self.additional_properties {
                    match additional {
                        Additional::Bool(allowed) => {
                            if !allowed && !uneval.is_empty() {
                                return error(
                                    "additionalProperties",
                                    kind!(AdditionalProperties, got: uneval.iter().cloned().cloned().collect()),
                                );
                            }
                        }
                        Additional::SchemaRef(sch) => {
                            for &pname in uneval.iter() {
                                if let Some(pvalue) = obj.get(pname) {
                                    schemas.get(*sch).validate(
                                        pvalue,
                                        format!("{vloc}/{}", escape(pname)),
                                        schemas,
                                    )?;
                                }
                            }
                        }
                    }
                    uneval.clear();
                }
            }
            Value::Array(arr) => {
                let Uneval::Items(uneval) = &mut uneval else {
                    unreachable!("array must value Uneval::Items"); 
                };
                if let Some(min) = self.min_items {
                    if arr.len() < min {
                        return error("minItems", kind!(MinItems, arr.len(), min));
                    }
                }
                if let Some(max) = self.max_items {
                    if arr.len() > max {
                        return error("maxItems", kind!(MaxItems, arr.len(), max));
                    }
                }
                if self.unique_items {
                    for i in 1..arr.len() {
                        for j in 0..i {
                            if arr[i] == arr[j] {
                                return error("uniqueItems", kind!(UniqueItems, got: [i, j]));
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
                        return error("minLength", kind!(MinLength, *len, min));
                    }
                }
                if let Some(max) = self.max_length {
                    let len = len.get_or_insert_with(|| s.chars().count());
                    if *len > max {
                        return error("maxLength", kind!(MaxLength, *len, max));
                    }
                }
                if let Some(regex) = &self.pattern {
                    if !regex.is_match(s) {
                        return error(
                            "pattern",
                            kind!(Pattern, s.clone(), regex.as_str().to_string()),
                        );
                    }
                }

                let mut decoded = Cow::from(s.as_bytes());
                if let Some(decode) = &self.decoder {
                    match decode(s) {
                        Some(bytes) => decoded = Cow::from(bytes),
                        None => {
                            return error(
                                "contentEncoding",
                                kind!(
                                    ContentEncoding,
                                    s.clone(),
                                    self.content_encoding.clone().unwrap()
                                ),
                            )
                        }
                    }
                }
                if let Some(media_type) = &self.media_type {
                    if !media_type(decoded.as_ref()) {
                        return error(
                            "contentMediaType",
                            kind!(
                                ContentMediaType,
                                decoded.into_owned(),
                                self.content_media_type.clone().unwrap()
                            ),
                        );
                    }
                }
            }
            Value::Number(n) => {
                if let Some(min) = &self.minimum {
                    if let (Some(minf), Some(vf)) = (min.as_f64(), n.as_f64()) {
                        if vf < minf {
                            return error("minimum", kind!(Minimum, n.clone(), min.clone()));
                        }
                    }
                }
                if let Some(max) = &self.maximum {
                    if let (Some(maxf), Some(vf)) = (max.as_f64(), n.as_f64()) {
                        if vf > maxf {
                            return error("maximum", kind!(Maximum, n.clone(), max.clone()));
                        }
                    }
                }
                if let Some(ex_min) = &self.exclusive_minimum {
                    if let (Some(ex_minf), Some(nf)) = (ex_min.as_f64(), n.as_f64()) {
                        if nf <= ex_minf {
                            return error(
                                "exclusiveMinimum",
                                kind!(ExclusiveMinimum, n.clone(), ex_min.clone()),
                            );
                        }
                    }
                }
                if let Some(ex_max) = &self.exclusive_maximum {
                    if let (Some(ex_maxf), Some(nf)) = (ex_max.as_f64(), n.as_f64()) {
                        if nf >= ex_maxf {
                            return error(
                                "exclusiveMaximum",
                                kind!(ExclusiveMaximum, n.clone(), ex_max.clone()),
                            );
                        }
                    }
                }
                if let Some(mul) = &self.multiple_of {
                    if let (Some(mulf), Some(nf)) = (mul.as_f64(), n.as_f64()) {
                        if (nf / mulf).fract() != 0.0 {
                            return error("multipleOf", kind!(MultipleOf, n.clone(), mul.clone()));
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(uneval)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Type {
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

    fn primitive(v: &Value) -> bool {
        !matches!(Self::of(v), Self::Array | Self::Object)
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Null => write!(f, "null"),
            Type::Bool => write!(f, "boolean"),
            Type::Number => write!(f, "number"),
            Type::Integer => write!(f, "integer"),
            Type::String => write!(f, "string"),
            Type::Array => write!(f, "array"),
            Type::Object => write!(f, "object"),
        }
    }
}

#[derive(Debug)]
pub struct ValidationError {
    pub absolute_keyword_location: String,
    pub instance_location: String,
    pub kind: ErrorKind,
}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "jsonschema: {} does not validate with {}: {}",
            quote(&self.instance_location),
            self.absolute_keyword_location,
            self.kind
        )
    }
}

#[derive(Debug)]
pub enum ErrorKind {
    Type { got: Type, want: Vec<Type> },
    Enum { got: Value, want: Vec<Value> },
    Const { got: Value, want: Value },
    MinProperties { got: usize, want: usize },
    MaxProperties { got: usize, want: usize },
    AdditionalProperties { got: Vec<String> },
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

impl Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // todo: use single quote for strings
        match self {
            Self::Type { got, want } => {
                // todo: why join not working for Type struct ??
                let want = join_iter(want, ", ");
                write!(f, "want {want}, but got {got}",)
            }
            Self::Enum { want, .. } => {
                if want.iter().all(Type::primitive) {
                    if want.len() == 1 {
                        write!(f, "value must be {want:?}")
                    } else {
                        let want = join_iter(want.iter().map(|e| format!("{e:?}")), " or ");
                        write!(f, "value must be one of {want}")
                    }
                } else {
                    write!(f, "enum failed")
                }
            }
            Self::Const { want, .. } => {
                if Type::primitive(want) {
                    write!(f, "value must be {want:?}")
                } else {
                    write!(f, "const failed")
                }
            }
            Self::MinProperties { got, want } => write!(
                f,
                "minimum {want} properties allowed, but got {got} properties"
            ),
            Self::MaxProperties { got, want } => write!(
                f,
                "maximum {want} properties allowed, but got {got} properties"
            ),
            Self::AdditionalProperties { got } => {
                write!(
                    f,
                    "additionalProperties {} not allowed",
                    join_iter(got.iter().map(quote), ", ")
                )
            }
            Self::Required { want } => write!(f, "missing properties {}", want.join(", ")),
            Self::DependentRequired { got, want } => write!(
                f,
                "properties {} required, if {} property exists",
                join_iter(want.iter().map(quote), ", "),
                quote(got)
            ),
            Self::MinItems { got, want } => {
                write!(f, "minimum {want} items allowed, but got {got} items")
            }
            Self::MaxItems { got, want } => {
                write!(f, "maximum {want} items allowed, but got {got} items")
            }
            Self::UniqueItems { got: [i, j] } => write!(f, "items at {i} and {j} are equal"),
            Self::MinLength { got, want } => write!(f, "length must be >={want}, but got {got}"),
            Self::MaxLength { got, want } => write!(f, "length must be <={want}, but got {got}"),
            Self::Pattern { got, want } => {
                write!(f, "{} does not match pattern {}", quote(got), quote(want))
            }
            Self::ContentEncoding { want, .. } => write!(f, "value is not {} encoded", quote(want)),
            Self::ContentMediaType { want, .. } => {
                write!(f, "value is not of mediatype {}", quote(want))
            }
            Self::Minimum { got, want } => write!(f, "must be >={want}, but got {got}"),
            Self::Maximum { got, want } => write!(f, "must be <={want}, but got {got}"),
            Self::ExclusiveMinimum { got, want } => write!(f, "must be > {want} but got {got}"),
            Self::ExclusiveMaximum { got, want } => write!(f, "must be < {want} but got {got}"),
            Self::MultipleOf { got, want } => write!(f, "{got} is not multipleOf {want}"),
        }
    }
}

type Decoder = Box<dyn Fn(&str) -> Option<Vec<u8>>>;
type MediaType = Box<dyn Fn(&[u8]) -> bool>;
