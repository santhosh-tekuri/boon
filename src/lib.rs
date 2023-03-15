/*! This crate supports JsonScehama validation for drafts `2020-12`, `2019-09`, `7`, `6` and `4`.

```rust,no_run
# use std::fs::File;
# use std::error::Error;
# use boon::*;
# use serde_json::Value;
# fn main() -> Result<(), Box<dyn Error>>{
let mut schemas = Schemas::new(); // container for compiled schemas
let mut compiler = Compiler::new();
let sch_index = compiler.compile("schema.json", &mut schemas)?;
let instance: Value = serde_json::from_reader(File::open("instance.json")?)?;
let valid = schemas.validate(&instance, sch_index).is_ok();
# Ok(())
# }
```

If schema file has no `$schema`, it assumes latest draft.
You can override this:
```rust,no_run
# use boon::*;
# let mut compiler = Compiler::new();
compiler.set_default_draft(Draft::V7);
```

The use of this option is HIGHLY encouraged to ensure continued
correct operation of your schema. The current default value will
not stay the same over time.

# Examples

- [example_from_strings]: loading schemas from Strings
- [example_from_https]: loading schemas from `http(s)`
- [example_custom_format]: registering custom format
- [example_custom_content_encoding]: registering custom contentEncoding
- [example_custom_content_media_type]: registering custom contentMediaType

# Compile Errors

```no_compile
println!("{compile_error}");
println!("{compile_error:#}"); // prints cause if any
```

Using alterate form in display will print cause if any.
This will be useful in cases like [`CompileError::LoadUrlError`],
as it would be useful to know whether the url does not exist or
the resource at url is not a valid json document.

# Validation Errors

[`ValidationError`] may have multiple `causes` resulting
in tree of errors.

`println!("{validation_error}")` prints:
```no_compile
jsonschema validation failed with file:///tmp/customer.json#
  at '': missing properties 'age'
  at '/billing_address': missing properties 'street_address', 'city', 'state'
```


The alternate form `println!("{validation_error:#}")` prints:
```no_compile
jsonschema validation failed with file:///tmp/customer.json#
  [I#] [S#/required] missing properties 'age'
  [I#/billing_address] [S#/properties/billing_address/$ref] validation failed with file:///tmp/address.json#
    [I#/billing_address] [S#/required] missing properties 'street_address', 'city', 'state'
```
here `I` refers to the instance document and `S` refers to last schema document.

for example:
- after line 1: `S` refers to `file:///tmp/customer.json`
- after line 3: `S` refers to `file://tmp/address.json`


# Output Formats

[`ValidationError`] can be converted into following output formats:
- [flag] `validation_error.flag_output()`
- [basic] `validation_error.basic_output()`
- [detailed] `validation_error.detailed_output()`

The output object implements `serde::Serialize`.

It also implement `Display` to print json:

```no_compile
println!("{output}"); // prints unformatted json
println!("{output:#}"); // prints indented json
```

[example_from_strings]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L22
[example_from_https]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L39
[example_from_yaml_files]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L64
[example_custom_format]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L97
[example_custom_content_encoding]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L129
[example_custom_content_media_type]: https://github.com/santhosh-tekuri/boon/blob/d466730e5e5c7c663bd6739e74e39d1e2f7baae4/tests/examples.rs#L171
[flag]: https://json-schema.org/draft/2020-12/json-schema-core.html#name-flag
[basic]: https://json-schema.org/draft/2020-12/json-schema-core.html#name-basic
[detailed]: https://json-schema.org/draft/2020-12/json-schema-core.html#name-detailed

*/

mod compiler;
mod content;
mod draft;
mod ecma;
mod formats;
mod loader;
mod output;
mod root;
mod roots;
mod util;
mod validator;

pub use compiler::Draft;
pub use compiler::*;
pub use content::*;
pub use formats::*;
pub use loader::*;

use std::{borrow::Cow, collections::HashMap, error::Error, fmt::Display};

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

    fn enqueue(&self, queue: &mut Vec<String>, mut loc: String) -> SchemaIndex {
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
        queue.push(loc);
        SchemaIndex(self.list.len() + queue.len() - 1)
    }

    fn insert(&mut self, locs: Vec<String>, compiled: Vec<Schema>) {
        for (loc, sch) in locs.into_iter().zip(compiled.into_iter()) {
            let i = self.list.len();
            self.list.push(sch);
            self.map.insert(loc, i);
        }
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

    /**
    Validates `v` with schema identified by `sch_index`

    # Panics

    Panics if `sch_index` is not generated for this instance.
    [`Schemas::contains`] can be used too ensure that it does not panic.
    */
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
    dynamic_ref: Option<DynamicRef>,
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
    format: Option<Format>,

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
    content_encoding: Option<Decoder>,
    content_media_type: Option<MediaType>,
    content_schema: Option<SchemaIndex>,

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

struct DynamicRef {
    sch: SchemaIndex,
    anchor: Option<String>,
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

impl Error for ValidationError {}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.display(f, 0)
    }
}

/// A list specifying general categories of validation errors.
#[derive(Debug)]
pub enum ErrorKind {
    Group,
    Schema {
        url: String,
    },
    ContentSchema,
    Reference {
        url: String,
    },
    RefCycle {
        url: String,
        kw_loc1: String,
        kw_loc2: String,
    },
    FalseSchema,
    Type {
        got: Type,
        want: Vec<Type>,
    },
    Enum {
        got: Value,
        want: Vec<Value>,
    },
    Const {
        got: Value,
        want: Value,
    },
    Format {
        got: Value,
        want: &'static str,
        err: Box<dyn Error>,
    },
    MinProperties {
        got: usize,
        want: usize,
    },
    MaxProperties {
        got: usize,
        want: usize,
    },
    AdditionalProperties {
        got: Vec<String>,
    },
    Required {
        want: Vec<String>,
    },
    Dependency {
        got: String,
        want: Vec<String>,
    },
    DependentRequired {
        got: String,
        want: Vec<String>,
    },
    DependentSchemas {
        got: String,
    },
    Items,
    MinItems {
        got: usize,
        want: usize,
    },
    MaxItems {
        got: usize,
        want: usize,
    },
    Contains,
    MinContains {
        got: Vec<usize>,
        want: usize,
    },
    MaxContains {
        got: Vec<usize>,
        want: usize,
    },
    UniqueItems {
        got: [usize; 2],
    },
    AdditionalItems {
        got: usize,
    },
    MinLength {
        got: usize,
        want: usize,
    },
    MaxLength {
        got: usize,
        want: usize,
    },
    Pattern {
        got: String,
        want: String,
    },
    ContentEncoding {
        got: String,
        want: &'static str,
        err: Box<dyn Error>,
    },
    ContentMediaType {
        got: Vec<u8>,
        want: &'static str,
        err: Box<dyn Error>,
    },
    Minimum {
        got: Number,
        want: Number,
    },
    Maximum {
        got: Number,
        want: Number,
    },
    ExclusiveMinimum {
        got: Number,
        want: Number,
    },
    ExclusiveMaximum {
        got: Number,
        want: Number,
    },
    MultipleOf {
        got: Number,
        want: Number,
    },
    Not,
    AllOf {
        subschema: Option<usize>,
    },
    AnyOf {
        subschema: Option<usize>,
    },
    OneOf(OneOf),
}

#[derive(Debug)]
pub enum OneOf {
    NoneMatch,
    Subschema(usize),
    MultiMatch(usize, usize),
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // todo: use single quote for strings
        match self {
            Self::Group => write!(f, "validation failed"),
            Self::Schema { url } => write!(f, "validation failed with {url}"),
            Self::ContentSchema => write!(f, "contentSchema failed"),
            Self::Reference { url } => {
                write!(f, "validation failed with {url}")
            }
            Self::RefCycle {
                url,
                kw_loc1,
                kw_loc2,
            } => write!(
                f,
                "both {} and {} resolve to {url} causing reference cycle",
                quote(kw_loc1),
                quote(kw_loc2)
            ),
            Self::FalseSchema => write!(f, "false schema"),
            Self::Type { got, want } => {
                // todo: why join not working for Type struct ??
                let want = join_iter(want, ", ");
                write!(f, "want {want}, but got {got}",)
            }
            Self::Enum { want, .. } => {
                if want.iter().all(Type::primitive) {
                    if want.len() == 1 {
                        write!(f, "value must be {}", want[0])
                    } else {
                        let want = join_iter(want.iter().map(|v| format!("{v}")), ", ");
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
            Self::Format { got, want, err } => write!(f, "{got} is not valid {want}: {err}"),
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
            Self::Dependency { got, want } => {
                if want.is_empty() {
                    write!(f, "dependency of property {} failed", quote(got))
                } else {
                    write!(
                        f,
                        "properties {} required, if {} property exists",
                        join_iter(want.iter().map(quote), ", "),
                        quote(got)
                    )
                }
            }
            Self::DependentRequired { got, want } => write!(
                f,
                "properties {} required, if {} property exists",
                join_iter(want.iter().map(quote), ", "),
                quote(got)
            ),
            Self::DependentSchemas { got } => {
                write!(f, "dependentSchema of property {} failed", quote(got))
            }
            Self::Items => write!(f, "items failed"),
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
            Self::AdditionalItems { got } => write!(f, "last {got} additionalItems not allowed"),
            Self::MinLength { got, want } => write!(f, "length must be >={want}, but got {got}"),
            Self::MaxLength { got, want } => write!(f, "length must be <={want}, but got {got}"),
            Self::Pattern { got, want } => {
                write!(f, "{} does not match pattern {}", quote(got), quote(want))
            }
            Self::ContentEncoding { want, err, .. } => {
                write!(f, "value is not {} encoded: {err}", quote(want))
            }
            Self::ContentMediaType { want, err, .. } => {
                write!(f, "value is not of mediatype {}: {err}", quote(want))
            }
            Self::Minimum { got, want } => write!(f, "must be >={want}, but got {got}"),
            Self::Maximum { got, want } => write!(f, "must be <={want}, but got {got}"),
            Self::ExclusiveMinimum { got, want } => write!(f, "must be > {want} but got {got}"),
            Self::ExclusiveMaximum { got, want } => write!(f, "must be < {want} but got {got}"),
            Self::MultipleOf { got, want } => write!(f, "{got} is not multipleOf {want}"),
            Self::Not => write!(f, "not failed"),
            Self::AllOf { subschema: None } => write!(f, "allOf failed",),
            Self::AllOf { subschema: Some(i) } => write!(f, "allOf subschema {i} failed",),
            Self::AnyOf { subschema: None } => write!(f, "anyOf failed, none matched"),
            Self::AnyOf { subschema: Some(i) } => write!(f, "anyOf subschema {i} failed"),
            Self::OneOf(OneOf::NoneMatch) => write!(f, "oneOf failed, none matched"),
            Self::OneOf(OneOf::Subschema(i)) => write!(f, "oneOf subschema {i} failed"),
            Self::OneOf(OneOf::MultiMatch(i, j)) => write!(f, "oneOf subschemas {i}, {j} matched"),
        }
    }
}
