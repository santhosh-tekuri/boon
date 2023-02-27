mod compiler;
mod content;
mod draft;
mod formats;
mod loader;
mod output;
mod root;
mod roots;
mod util;
mod validator;

pub use compiler::Draft;
pub use compiler::*;
use content::{Decoder, MediaType};
use formats::Format;
pub use loader::*;

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    error::Error,
    fmt::Display,
};

use regex::Regex;
use serde_json::{Number, Value};
use util::*;

/// Identifier to compiled schema.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaIndex(usize);

/// Collection of compiled schemas.
#[derive(Default)]
pub struct Schemas {
    list: Vec<Schema>,
    map: HashMap<String, usize>, // loc => schema-index
}

impl Schemas {
    pub fn new() -> Self {
        Self::default()
    }

    fn enqueue(&self, queue: &mut VecDeque<String>, mut loc: String) -> SchemaIndex {
        if loc.rfind('#').is_none() {
            loc.push('#');
        }

        if let Some(&index) = self.map.get(&loc) {
            // already got compiled
            return SchemaIndex(index);
        }
        if let Some(qindex) = queue.iter().position(|e| *e == loc) {
            // already queued for compilation
            return SchemaIndex(self.list.len() + qindex);
        }

        // new compilation request
        queue.push_back(loc);
        SchemaIndex(self.list.len() + queue.len() - 1)
    }

    fn insert(&mut self, loc: String, sch: Schema) -> SchemaIndex {
        let index = self.list.len();
        self.list.push(sch);
        self.map.insert(loc, index);
        SchemaIndex(index)
    }

    fn get(&self, idx: SchemaIndex) -> &Schema {
        &self.list[idx.0] // todo: return bug
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

    /// Returns true if `sch_index` is generated for this instance.
    pub fn contains(&self, sch_index: SchemaIndex) -> bool {
        self.list.get(sch_index.0).is_some()
    }

    /// Validates `v` with schema identified by `sch_index`
    ///
    /// # Panics
    ///
    /// Panics if `sch_index` is not generated for this instance.
    /// [`Schemas::contains`] can be used too ensure that it does not panic.
    pub fn validate(&self, v: &Value, sch_index: SchemaIndex) -> Result<(), ValidationError> {
        let Some(sch) = self.list.get(sch_index.0) else {
            panic!("Schemas::validate: schema index out of bounds");
        };
        validator::validate(v, sch, self)
    }
}

#[derive(Default)]
struct Schema {
    draft_version: usize,
    idx: SchemaIndex,
    loc: String,
    resource: SchemaIndex,
    dynamic_anchors: HashMap<String, SchemaIndex>,

    // type agnostic --
    boolean: Option<bool>, // boolean schema
    ref_: Option<SchemaIndex>,
    recursive_ref: Option<SchemaIndex>,
    recursive_anchor: bool,
    dynamic_ref: Option<SchemaIndex>,
    dynamic_anchor: Option<String>,
    types: Vec<Type>,
    enum_: Vec<Value>,
    constant: Option<Value>,
    not: Option<SchemaIndex>,
    all_of: Vec<SchemaIndex>,
    any_of: Vec<SchemaIndex>,
    one_of: Vec<SchemaIndex>,
    if_: Option<SchemaIndex>,
    then: Option<SchemaIndex>,
    else_: Option<SchemaIndex>,
    format: Option<(String, Format)>,

    // object --
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    required: Vec<String>,
    properties: HashMap<String, SchemaIndex>,
    pattern_properties: Vec<(Regex, SchemaIndex)>,
    property_names: Option<SchemaIndex>,
    additional_properties: Option<Additional>,
    dependent_required: HashMap<String, Vec<String>>,
    dependent_schemas: HashMap<String, SchemaIndex>,
    dependencies: HashMap<String, Dependency>,
    unevaluated_properties: Option<SchemaIndex>,

    // array --
    min_items: Option<usize>,
    max_items: Option<usize>,
    unique_items: bool,
    min_contains: Option<usize>,
    max_contains: Option<usize>,
    contains: Option<SchemaIndex>,
    items: Option<Items>,
    additional_items: Option<Additional>,
    prefix_items: Vec<SchemaIndex>,
    items2020: Option<SchemaIndex>,
    unevaluated_items: Option<SchemaIndex>,

    // string --
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Regex>,
    content_encoding: Option<(String, Decoder)>,
    content_media_type: Option<(String, MediaType)>,

    // number --
    minimum: Option<Number>,
    maximum: Option<Number>,
    exclusive_minimum: Option<Number>,
    exclusive_maximum: Option<Number>,
    multiple_of: Option<Number>,
}

#[derive(Debug)]
enum Items {
    SchemaRef(SchemaIndex),
    SchemaRefs(Vec<SchemaIndex>),
}

#[derive(Debug)]
enum Additional {
    Bool(bool),
    SchemaRef(SchemaIndex),
}

#[derive(Debug)]
enum Dependency {
    Props(Vec<String>),
    SchemaRef(SchemaIndex),
}

impl Schema {
    fn new(loc: String) -> Self {
        Self {
            loc,
            ..Default::default()
        }
    }
}

/// JSON data types for JSONSchema
#[derive(Debug, PartialEq, Clone)]
pub enum Type {
    Null,
    Bool,
    Number,
    /// Matches any number with a zero fractional part
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

/// Error type for validation failures.
#[derive(Debug)]
pub struct ValidationError {
    /// The relative location of the validating keyword that follows the validation path
    pub keyword_location: String,
    /// The absolute, dereferenced location of the validating keyword
    pub absolute_keyword_location: String,
    /// The location of the JSON value within the instance being validated
    pub instance_location: String,
    /// kind of error
    pub kind: ErrorKind,
    /// Holds nested errors
    pub causes: Vec<ValidationError>,
}

impl ValidationError {
    fn print_alternate<'a>(
        &'a self,
        f: &mut std::fmt::Formatter,
        inst_loc: &'a str,
        mut sch_loc: &'a str,
        indent: usize,
    ) -> std::fmt::Result {
        for _ in 0..indent {
            write!(f, "  ")?;
        }
        if let ErrorKind::Schema { .. } = &self.kind {
            write!(f, "jsonschema {}", self.kind)?;
        } else {
            if f.sign_minus() {
                let inst_ptr = Loc::locate(inst_loc, &self.instance_location);
                let sch_ptr = Loc::locate(sch_loc, &self.absolute_keyword_location);
                write!(f, "I[{inst_ptr}] S[{sch_ptr}] ")?;
            } else {
                let inst_ptr = &self.instance_location;
                let (_, sch_ptr) = split(&self.absolute_keyword_location);
                write!(f, "I[{inst_ptr}] S[{sch_ptr}] ")?;
            }
            if let ErrorKind::Reference { url } = &self.kind {
                let (a, _) = split(sch_loc);
                let (b, ptr) = split(url);
                if a == b {
                    write!(f, "validation failed with {ptr}")?;
                } else {
                    write!(f, "{}", self.kind)?;
                }
            } else {
                write!(f, "{}", self.kind)?;
            }
            // NOTE: this code used to check relative path correctness
            // let (_, ptr) = split(&self.absolute_keyword_location);
            // write!(
            //     f,
            //     "[I{inst_ptr}] [I{}] [S{sch_ptr}] [S{}]{}",
            //     self.instance_location, ptr, self.kind
            // )?;
        }
        sch_loc = if let ErrorKind::Reference { url } = &self.kind {
            url
        } else {
            &self.absolute_keyword_location
        };
        for cause in &self.causes {
            writeln!(f)?;
            cause.print_alternate(f, &self.instance_location, sch_loc, indent + 1)?;
        }
        Ok(())
    }
}

impl Error for ValidationError {}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            // NOTE: only root validationError supports altername display
            if let ErrorKind::Schema { url } = &self.kind {
                return self.print_alternate(f, "", url, 0);
            }
        }

        // non-alternate --
        fn fmt_leaves(
            e: &ValidationError,
            f: &mut std::fmt::Formatter,
            mut newline: bool,
        ) -> Result<bool, std::fmt::Error> {
            if e.causes.is_empty() {
                if newline {
                    writeln!(f)?;
                }
                write!(f, "  at {}: {}", quote(&e.instance_location), &e.kind)?;
                newline = true;
            } else {
                for cause in &e.causes {
                    newline = fmt_leaves(cause, f, newline)?;
                }
            }
            Ok(newline)
        }
        writeln!(
            f,
            "jsonschema validation failed with {}",
            &self.absolute_keyword_location
        )?;
        fmt_leaves(self, f, false).map(|_| ())

        // let mut leaf = self;
        // while let [cause, ..] = leaf.causes.as_slice() {
        //     leaf = cause;
        // }
        // if leaf.instance_location.is_empty() {
        //     write!(
        //         f,
        //         "jsonschema: validation failed with {}",
        //         &leaf.absolute_keyword_location
        //     )
        // } else {
        //     write!(
        //         f,
        //         "jsonschema: {} does not validate with {}: {}",
        //         &leaf.instance_location, &leaf.absolute_keyword_location, &leaf.kind
        //     )
        // }
    }
}

/// A list specifying general categories of validation errors.
#[derive(Debug)]
pub enum ErrorKind {
    Group,
    Schema { url: String },
    Reference { url: String },
    RefCycle,
    FalseSchema,
    Type { got: Type, want: Vec<Type> },
    Enum { got: Value, want: Vec<Value> },
    Const { got: Value, want: Value },
    Format { got: Value, want: String },
    MinProperties { got: usize, want: usize },
    MaxProperties { got: usize, want: usize },
    AdditionalProperties { got: Vec<String> },
    Required { want: Vec<String> },
    DependentRequired { got: String, want: Vec<String> },
    MinItems { got: usize, want: usize },
    MaxItems { got: usize, want: usize },
    Contains,
    MinContains { got: Vec<usize>, want: usize },
    MaxContains { got: Vec<usize>, want: usize },
    UniqueItems { got: [usize; 2] },
    AdditionalItems { got: usize },
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
    Not,
    AllOf { got: Vec<usize> },
    AnyOf,
    OneOf { got: Vec<usize> },
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // todo: use single quote for strings
        match self {
            Self::Group => write!(f, "validation failed"),
            Self::Schema { url } => write!(f, "validation failed with {url}"),
            Self::Reference { url } => write!(f, "validation failed with {url}"),
            Self::RefCycle => write!(f, "reference cycle detected"),
            Self::FalseSchema => write!(f, "false schema"),
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
                    write!(f, "value must be {want}")
                } else {
                    write!(f, "const failed")
                }
            }
            Self::Format { got, want } => write!(f, "{got} is not valid {want}"),
            Self::MinProperties { got, want } => write!(
                f,
                "minimum {want} properties required, but got {got} properties"
            ),
            Self::MaxProperties { got, want } => write!(
                f,
                "maximum {want} properties required, but got {got} properties"
            ),
            Self::AdditionalProperties { got } => {
                write!(
                    f,
                    "additionalProperties {} not allowed",
                    join_iter(got.iter().map(quote), ", ")
                )
            }
            Self::Required { want } => write!(
                f,
                "missing properties {}",
                join_iter(want.iter().map(quote), ", ")
            ),
            Self::DependentRequired { got, want } => write!(
                f,
                "properties {} required, if {} property exists",
                join_iter(want.iter().map(quote), ", "),
                quote(got)
            ),
            Self::MinItems { got, want } => {
                write!(f, "minimum {want} items required, but got {got} items")
            }
            Self::MaxItems { got, want } => {
                write!(f, "maximum {want} items required, but got {got} items")
            }
            Self::MinContains { got, want } => {
                if got.is_empty() {
                    write!(
                        f,
                        "minimum {want} items required to match contains schema, but found none",
                    )
                } else {
                    write!(
                        f,
                        "minimum {want} items required to match contains schema, but found {} items at {}",
                        got.len(),
                        join_iter(got, ", ")
                    )
                }
            }
            Self::Contains => write!(f, "no items match contains schema"),
            Self::MaxContains { got, want } => {
                write!(
                        f,
                        "maximum {want} items required to match contains schema, but found {} items at {}",
                        got.len(),
                        join_iter(got, ", ")
                    )
            }
            Self::UniqueItems { got: [i, j] } => write!(f, "items at {i} and {j} are equal"),
            Self::AdditionalItems { got } => write!(f, "got {got} additionalItems"),
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
            Self::Not => write!(f, "not failed"),
            Self::AllOf { got } => write!(
                f,
                "allOf failed, subschemas {} did not match",
                join_iter(got, ", ")
            ),
            Self::AnyOf => write!(f, "anyOf failed, none matched"),
            Self::OneOf { got } => {
                if got.is_empty() {
                    write!(f, "oneOf failed, none matched")
                } else {
                    write!(
                        f,
                        "want valid against oneOf subschema, but valid against subschemas {}",
                        join_iter(got, " and "),
                    )
                }
            }
        }
    }
}
