mod compiler;
mod content;
mod draft;
mod formats;
mod loader;
mod output;
mod root;
mod roots;
mod util;

pub use compiler::Draft;
pub use compiler::*;
use content::{Decoder, MediaType};
use formats::Format;
pub use loader::*;

use std::{
    borrow::Cow,
    cmp::min,
    collections::{HashMap, HashSet, VecDeque},
    fmt::Display,
};

use regex::Regex;
use serde_json::{Number, Value};
use util::*;

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaIndex(usize);

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
        let scope = Scope {
            sch: sch.idx,
            kw_path: Cow::from(""),
            vid: 0,
            parent: None,
        };
        match sch.validate(v, String::new(), self, scope) {
            Err(e) => {
                let mut err = ValidationError {
                    keyword_location: String::new(),
                    absolute_keyword_location: sch.loc.clone(),
                    instance_location: String::new(),
                    kind: ErrorKind::Schema {
                        url: sch.loc.clone(),
                    },
                    causes: vec![],
                };
                if let ErrorKind::Group = e.kind {
                    err.causes = e.causes;
                } else {
                    err.causes.push(e);
                }
                Err(err)
            }
            Ok(_) => Ok(()),
        }
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
    ($kind: ident) => {
        ErrorKind::$kind
    };
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

#[derive(Default)]
struct Uneval<'v> {
    props: HashSet<&'v String>,
    items: HashSet<usize>,
}

impl<'v> Uneval<'v> {
    fn merge(&mut self, other: &Uneval) {
        self.props.retain(|p| other.props.contains(p));
        self.items.retain(|i| other.items.contains(i));
    }
}

impl<'v> From<&'v Value> for Uneval<'v> {
    fn from(v: &'v Value) -> Self {
        let mut uneval = Self::default();
        match v {
            Value::Object(obj) => uneval.props = obj.keys().collect(),
            Value::Array(arr) => uneval.items = (0..arr.len()).collect(),
            _ => (),
        }
        uneval
    }
}

#[derive(Debug, Default)]
struct Scope<'a> {
    sch: SchemaIndex,
    kw_path: Cow<'static, str>,
    /// unique id of value being validated
    // if two scope validate same value, they will have same vid
    vid: usize,
    parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    fn child(sch: SchemaIndex, kw_path: Cow<'static, str>, vid: usize, parent: &'a Scope) -> Self {
        Self {
            sch,
            kw_path,
            vid,
            parent: Some(parent),
        }
    }

    fn kw_loc(&self, kw_path: &str) -> String {
        let mut loc = kw_path.to_string();
        let mut scope = self;
        loop {
            if !loc.is_empty() {
                loc.insert(0, '/');
            }
            loc.insert_str(0, scope.kw_path.as_ref());
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                break;
            }
        }
        loc
    }

    fn has_cycle(&self) -> bool {
        let mut scope = self.parent;
        while let Some(scp) = scope {
            if scp.vid != self.vid {
                break;
            }
            if scp.sch == self.sch {
                return true;
            }
            scope = scp.parent;
        }
        false
    }
}

impl Schema {
    fn new(loc: String) -> Self {
        Self {
            loc,
            ..Default::default()
        }
    }

    fn validate<'v>(
        &self,
        v: &'v Value,
        vloc: String,
        schemas: &Schemas,
        scope: Scope,
    ) -> Result<Uneval<'v>, ValidationError> {
        let mut h = Helper {
            v,
            schemas,
            scope,
            sloc: &self.loc,
            vloc,
            errors: vec![],
        };

        if h.scope.has_cycle() {
            return Err(h.error("", kind!(RefCycle)));
        }

        let mut uneval = Uneval::from(v);

        // boolean --
        if let Some(b) = self.boolean {
            if !b {
                return Err(h.error("", kind!(FalseSchema)));
            }
            return Ok(uneval);
        }

        // type --
        if !self.types.is_empty() {
            let v_type = Type::of(v);
            let matched = self.types.iter().any(|t| {
                if *t == Type::Integer && v_type == Type::Number {
                    if let Value::Number(n) = v {
                        return n.is_i64()
                            || n.is_u64()
                            || n.as_f64().filter(|n| n.fract() == 0.0).is_some();
                    }
                }
                *t == v_type
            });
            if !matched {
                h.add_error("type", kind!(Type, v_type, self.types.clone()));
            }
        }

        // enum --
        if !self.enum_.is_empty() && !self.enum_.iter().any(|e| equals(e, v)) {
            h.add_error("enum", kind!(Enum, v.clone(), self.enum_.clone()));
        }

        // constant --
        if let Some(c) = &self.constant {
            if !equals(v, c) {
                h.add_error("const", kind!(Const, v.clone(), c.clone()));
            }
        }

        // format --
        if let Some((format, check)) = &self.format {
            if !check(v) {
                h.add_error("format", kind!(Format, v.clone(), format.clone()));
            }
        }

        self.validate_object(&mut h, &mut uneval);
        self.validate_array(&mut h, &mut uneval);
        self.validate_string(&mut h);
        self.validate_number(&mut h);

        self.validate_references(&mut h, &mut uneval);
        self.validate_conditional(&mut h, &mut uneval);
        self.validate_unevaluated(&mut h, &mut uneval);

        h.result(uneval)
    }

    fn validate_references(&self, h: &mut Helper, uneval: &mut Uneval) {
        // $ref --
        if let Some(ref_) = self.ref_ {
            h.add_err(h.validate_ref(ref_, "$ref", uneval));
        }

        // $recursiveRef --
        if let Some(mut sch) = self.recursive_ref {
            if h.schema(sch).recursive_anchor {
                sch = h.resolve_recursive_anchor().unwrap_or(sch);
            }
            h.add_err(h.validate_ref(sch, "$recursiveRef", uneval));
        }

        // $dynamicRef --
        if let Some(mut sch) = self.dynamic_ref {
            if let Some(name) = &h.schema(sch).dynamic_anchor {
                sch = h.resolve_dynamic_anchor(name).unwrap_or(sch);
            }
            h.add_err(h.validate_ref(sch, "$dynamicRef", uneval));
        }
    }

    fn validate_conditional(&self, h: &mut Helper, uneval: &mut Uneval) {
        // not --
        if let Some(not) = self.not {
            if h.validate_self(not, "not".into(), uneval).is_ok() {
                h.add_error("not", kind!(Not));
            }
        }

        // allOf --
        if !self.all_of.is_empty() {
            let (mut failed, mut allof_errors) = (vec![], vec![]);
            for (i, sch) in self.all_of.iter().enumerate() {
                let kw_path = format!("allOf/{i}");
                if let Err(e) = h.validate_self(*sch, kw_path.into(), uneval) {
                    failed.push(i);
                    allof_errors.push(e);
                }
            }
            if !failed.is_empty() {
                h.add_errors(allof_errors, "allOf", kind!(AllOf, got: failed));
            }
        }

        // anyOf --
        if !self.any_of.is_empty() {
            // NOTE: all schemas must be checked
            let mut anyof_errors = vec![];
            for (i, sch) in self.any_of.iter().enumerate() {
                let kw_path = format!("anyOf/{i}");
                if let Err(e) = h.validate_self(*sch, kw_path.into(), uneval) {
                    anyof_errors.push(e);
                }
            }
            if anyof_errors.len() == self.any_of.len() {
                // none matched
                h.add_errors(anyof_errors, "anyOf", kind!(AnyOf));
            }
        }

        // oneOf --
        if !self.one_of.is_empty() {
            let (mut matched, mut oneof_errors) = (vec![], vec![]);
            for (i, sch) in self.one_of.iter().enumerate() {
                let kw_path = format!("oneOf/{i}");
                if let Err(e) = h.validate_self(*sch, kw_path.into(), uneval) {
                    oneof_errors.push(e);
                } else {
                    matched.push(i);
                    if matched.len() == 2 {
                        break;
                    }
                }
            }
            if matched.is_empty() {
                // none matched
                h.add_errors(oneof_errors, "oneOf", kind!(OneOf, got: matched));
            } else if matched.len() > 1 {
                h.add_error("oneOf", kind!(OneOf, got: matched));
            }
        }

        // if, then, else --
        if let Some(if_) = self.if_ {
            if h.validate_self(if_, "if".into(), uneval).is_ok() {
                if let Some(then) = self.then {
                    h.add_err(h.validate_self(then, "then".into(), uneval));
                }
            } else if let Some(else_) = self.else_ {
                h.add_err(h.validate_self(else_, "else".into(), uneval));
            }
        }
    }

    fn validate_unevaluated(&self, h: &mut Helper, uneval: &mut Uneval) {
        // unevaluatedProps --
        if let (Some(sch), Value::Object(obj)) = (self.unevaluated_properties, h.v) {
            for pname in &uneval.props {
                if let Some(pvalue) = obj.get(*pname) {
                    let kw_path = "unevaluatedProperties";
                    h.add_err(h.validate(sch, kw_path.into(), pvalue, &escape(pname)));
                }
            }
            uneval.props.clear();
        }

        // unevaluatedItems --
        if let (Some(sch), Value::Array(arr)) = (self.unevaluated_items, h.v) {
            for i in &uneval.items {
                if let Some(pvalue) = arr.get(*i) {
                    let kw_path = "unevaluatedItems";
                    h.add_err(h.validate(sch, kw_path.into(), pvalue, &i.to_string()));
                }
            }
            uneval.items.clear();
        }
    }

    fn validate_object(&self, h: &mut Helper, uneval: &mut Uneval) {
        let Value::Object(obj) = h.v else {
            return;
        };

        // minProperties --
        if let Some(min) = self.min_properties {
            if obj.len() < min {
                h.add_error("minProperties", kind!(MinProperties, obj.len(), min));
            }
        }

        // maxProperties --
        if let Some(max) = self.max_properties {
            if obj.len() > max {
                h.add_error("maxProperties", kind!(MaxProperties, obj.len(), max));
            }
        }

        // required --
        let missing = self
            .required
            .iter()
            .filter(|p| !obj.contains_key(p.as_str()))
            .cloned()
            .collect::<Vec<String>>();
        if !missing.is_empty() {
            h.add_error("required", kind!(Required, want: missing));
        }

        // dependencies --
        for (pname, dependency) in &self.dependencies {
            if obj.contains_key(pname) {
                let kw_path = format!("dependencies/{}", escape(pname));
                match dependency {
                    Dependency::Props(required) => {
                        let missing = required
                            .iter()
                            .filter(|p| !obj.contains_key(p.as_str()))
                            .cloned()
                            .collect::<Vec<String>>();
                        if !missing.is_empty() {
                            h.add_error(&kw_path, kind!(DependentRequired, pname.clone(), missing));
                        }
                    }
                    Dependency::SchemaRef(sch) => {
                        h.add_err(h.validate_self(*sch, kw_path.into(), uneval));
                    }
                }
            }
        }

        // dependentSchemas --
        for (pname, sch) in &self.dependent_schemas {
            if obj.contains_key(pname) {
                let kw_path = format!("dependentSchemas/{}", escape(pname));
                h.add_err(h.validate_self(*sch, kw_path.into(), uneval));
            }
        }

        // dependentRequired --
        for (pname, required) in &self.dependent_required {
            if obj.contains_key(pname) {
                let missing = required
                    .iter()
                    .filter(|p| !obj.contains_key(p.as_str()))
                    .cloned()
                    .collect::<Vec<String>>();
                if !missing.is_empty() {
                    let kind = kind!(DependentRequired, pname.clone(), missing);
                    h.add_error(&format!("dependentRequired/{}", escape(pname)), kind);
                }
            }
        }

        // properties --
        for (pname, &psch) in &self.properties {
            if let Some(pvalue) = obj.get(pname) {
                uneval.props.remove(pname);
                let kw_path = format!("properties/{}", escape(pname));
                h.add_err(h.validate(psch, kw_path.into(), pvalue, &escape(pname)));
            }
        }

        // patternProperties --
        for (regex, psch) in &self.pattern_properties {
            for (pname, pvalue) in obj.iter().filter(|(pname, _)| regex.is_match(pname)) {
                uneval.props.remove(pname);
                let kw_path = format!("patternProperties/{}", escape(regex.as_str()));
                h.add_err(h.validate(*psch, kw_path.into(), pvalue, &escape(pname)));
            }
        }

        // propertyNames --
        if let Some(sch) = &self.property_names {
            for pname in obj.keys() {
                let v = Value::String(pname.to_owned());
                h.add_err(h.validate(*sch, "propertyNames".into(), &v, &escape(pname)));
            }
        }

        // additionalProperties --
        if let Some(additional) = &self.additional_properties {
            let kw_path = "additionalProperties";
            match additional {
                Additional::Bool(allowed) => {
                    if !allowed && !uneval.props.is_empty() {
                        let kind = kind!(AdditionalProperties, got: uneval.props.iter().cloned().cloned().collect());
                        h.add_error(kw_path, kind);
                    }
                }
                Additional::SchemaRef(sch) => {
                    for &pname in uneval.props.iter() {
                        if let Some(pvalue) = obj.get(pname) {
                            h.add_err(h.validate(*sch, kw_path.into(), pvalue, &escape(pname)));
                        }
                    }
                }
            }
            uneval.props.clear();
        }
    }

    fn validate_array(&self, h: &mut Helper, uneval: &mut Uneval) {
        let Value::Array(arr) = h.v else {
            return;
        };

        // minItems --
        if let Some(min) = self.min_items {
            if arr.len() < min {
                h.add_error("minItems", kind!(MinItems, arr.len(), min));
            }
        }

        // maxItems --
        if let Some(max) = self.max_items {
            if arr.len() > max {
                h.add_error("maxItems", kind!(MaxItems, arr.len(), max));
            }
        }

        // uniqueItems --
        if self.unique_items {
            for i in 1..arr.len() {
                for j in 0..i {
                    if equals(&arr[i], &arr[j]) {
                        h.add_error("uniqueItems", kind!(UniqueItems, got: [j, i]));
                    }
                }
            }
        }

        // items --
        if let Some(items) = &self.items {
            match items {
                Items::SchemaRef(sch) => {
                    for (i, item) in arr.iter().enumerate() {
                        h.add_err(h.validate(*sch, "items".into(), item, &i.to_string()));
                    }
                    uneval.items.clear();
                }
                Items::SchemaRefs(list) => {
                    for (i, (item, sch)) in arr.iter().zip(list).enumerate() {
                        uneval.items.remove(&i);
                        let kw_path = format!("items/{i}");
                        h.add_err(h.validate(*sch, kw_path.into(), item, &i.to_string()));
                    }
                }
            }
        }

        // additionalItems --
        if let Some(additional) = &self.additional_items {
            let kw_path = "additionalItems";
            match additional {
                Additional::Bool(allowed) => {
                    if !allowed && !uneval.items.is_empty() {
                        h.add_error(
                            kw_path,
                            kind!(AdditionalItems, got: arr.len() - uneval.items.len()),
                        );
                    }
                }
                Additional::SchemaRef(sch) => {
                    let from = arr.len() - uneval.items.len();
                    for (i, item) in arr[from..].iter().enumerate() {
                        h.add_err(h.validate(*sch, kw_path.into(), item, &i.to_string()));
                    }
                }
            }
            uneval.items.clear();
        }

        // prefixItems --
        for (i, (sch, item)) in self.prefix_items.iter().zip(arr).enumerate() {
            uneval.items.remove(&i);
            let kw_path = format!("prefixItems/{i}");
            h.add_err(h.validate(*sch, kw_path.into(), item, &i.to_string()));
        }

        // items2020 --
        if let Some(sch) = &self.items2020 {
            let from = min(arr.len(), self.prefix_items.len());
            for (i, item) in arr[from..].iter().enumerate() {
                h.add_err(h.validate(*sch, "items".into(), item, &i.to_string()));
            }
            uneval.items.clear();
        }

        // contains --
        let mut contains_matched = vec![];
        let mut contains_errors = vec![];
        if let Some(sch) = &self.contains {
            for (i, item) in arr.iter().enumerate() {
                if let Err(e) = h.validate(*sch, "contains".into(), item, &i.to_string()) {
                    contains_errors.push(e);
                } else {
                    contains_matched.push(i);
                    if self.draft_version >= 2020 {
                        uneval.items.remove(&i);
                    }
                }
            }
        }

        // minContains --
        if let Some(min) = self.min_contains {
            if contains_matched.len() < min {
                let kind = kind!(MinContains, contains_matched.clone(), min);
                let mut e = h.error("minContains", kind);
                e.causes = contains_errors;
                h.errors.push(e);
            }
        } else if self.contains.is_some() && contains_matched.is_empty() {
            let mut e = h.error("contains", kind!(Contains));
            e.causes = contains_errors;
            h.errors.push(e);
        }

        // maxContains --
        if let Some(max) = self.max_contains {
            if contains_matched.len() > max {
                h.add_error("maxContains", kind!(MaxContains, contains_matched, max));
            }
        }
    }

    fn validate_string(&self, h: &mut Helper) {
        let Value::String(s) = h.v else {
            return;
        };

        let mut len = None;

        // minLength --
        if let Some(min) = self.min_length {
            let len = len.get_or_insert_with(|| s.chars().count());
            if *len < min {
                h.add_error("minLength", kind!(MinLength, *len, min));
            }
        }

        // maxLength --
        if let Some(max) = self.max_length {
            let len = len.get_or_insert_with(|| s.chars().count());
            if *len > max {
                h.add_error("maxLength", kind!(MaxLength, *len, max));
            }
        }

        // pattern --
        if let Some(regex) = &self.pattern {
            if !regex.is_match(s) {
                let kind = kind!(Pattern, s.clone(), regex.as_str().to_string());
                h.add_error("pattern", kind);
            }
        }

        // contentEncoding --
        let mut decoded = Cow::from(s.as_bytes());
        if let Some((encoding, decode)) = &self.content_encoding {
            match decode(s) {
                Some(bytes) => decoded = Cow::from(bytes),
                None => {
                    let kind = kind!(ContentEncoding, s.clone(), encoding.clone());
                    h.add_error("contentEncoding", kind)
                }
            }
        }

        // contentMediaType --
        if let Some((media_type, check)) = &self.content_media_type {
            if !check(decoded.as_ref()) {
                let kind = kind!(ContentMediaType, decoded.into_owned(), media_type.clone());
                h.add_error("contentMediaType", kind);
            }
        }
    }

    fn validate_number(&self, h: &mut Helper) {
        let Value::Number(n) = h.v else {
            return;
        };

        // minimum --
        if let Some(min) = &self.minimum {
            if let (Some(minf), Some(vf)) = (min.as_f64(), n.as_f64()) {
                if vf < minf {
                    h.add_error("minimum", kind!(Minimum, n.clone(), min.clone()));
                }
            }
        }

        // maximum --
        if let Some(max) = &self.maximum {
            if let (Some(maxf), Some(vf)) = (max.as_f64(), n.as_f64()) {
                if vf > maxf {
                    h.add_error("maximum", kind!(Maximum, n.clone(), max.clone()));
                }
            }
        }

        // exclusiveMinimum --
        if let Some(ex_min) = &self.exclusive_minimum {
            if let (Some(ex_minf), Some(nf)) = (ex_min.as_f64(), n.as_f64()) {
                if nf <= ex_minf {
                    let kind = kind!(ExclusiveMinimum, n.clone(), ex_min.clone());
                    h.add_error("exclusiveMinimum", kind);
                }
            }
        }

        // exclusiveMaximum --
        if let Some(ex_max) = &self.exclusive_maximum {
            if let (Some(ex_maxf), Some(nf)) = (ex_max.as_f64(), n.as_f64()) {
                if nf >= ex_maxf {
                    let kind = kind!(ExclusiveMaximum, n.clone(), ex_max.clone());
                    h.add_error("exclusiveMaximum", kind);
                }
            }
        }

        // multipleOf --
        if let Some(mul) = &self.multiple_of {
            if let (Some(mulf), Some(nf)) = (mul.as_f64(), n.as_f64()) {
                if (nf / mulf).fract() != 0.0 {
                    h.add_error("multipleOf", kind!(MultipleOf, n.clone(), mul.clone()));
                }
            }
        }
    }
}

struct Helper<'v, 'a, 'b, 'c> {
    v: &'v Value,
    schemas: &'a Schemas,
    scope: Scope<'b>,
    sloc: &'c String,
    vloc: String,
    errors: Vec<ValidationError>,
}

impl<'v, 'a, 'b, 'c> Helper<'v, 'a, 'b, 'c> {
    fn schema(&self, sch: SchemaIndex) -> &Schema {
        self.schemas.get(sch)
    }

    fn error(&self, kw_path: &str, kind: ErrorKind) -> ValidationError {
        ValidationError {
            keyword_location: self.scope.kw_loc(kw_path),
            absolute_keyword_location: match kw_path.is_empty() {
                true => self.sloc.clone(),
                false => format!("{}/{kw_path}", self.sloc),
            },
            instance_location: self.vloc.clone(),
            kind,
            causes: vec![],
        }
    }

    fn add_error(&mut self, kw_path: &str, kind: ErrorKind) {
        self.errors.push(self.error(kw_path, kind));
    }

    fn add_err(&mut self, result: Result<(), ValidationError>) {
        self.errors.extend(result.err().into_iter());
    }

    fn add_errors(&mut self, errors: Vec<ValidationError>, kw_path: &str, kind: ErrorKind) {
        if errors.len() == 1 {
            self.errors.extend(errors);
        } else {
            let mut err = self.error(kw_path, kind);
            err.causes = errors;
            self.errors.push(err);
        }
    }

    fn result(mut self, uneval: Uneval) -> Result<Uneval, ValidationError> {
        match self.errors.len() {
            0 => Ok(uneval),
            1 => Err(self.errors.remove(0)),
            _ => {
                let mut e = self.error("", kind!(Group));
                e.causes = self.errors;
                Err(e)
            }
        }
    }

    fn validate(
        &self,
        sch: SchemaIndex,
        kw_path: Cow<'static, str>,
        v: &Value,
        vpath: &str,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, kw_path, self.scope.vid + 1, &self.scope);
        self.schemas
            .get(sch)
            .validate(v, format!("{}/{vpath}", self.vloc), self.schemas, scope)
            .map(|_| ())
    }

    fn validate_self(
        &self,
        sch: SchemaIndex,
        kw_path: Cow<'static, str>,
        uneval: &mut Uneval<'_>,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, kw_path, self.scope.vid, &self.scope);
        let result = self
            .schemas
            .get(sch)
            .validate(self.v, self.vloc.clone(), self.schemas, scope);
        if let Ok(reply) = &result {
            uneval.merge(reply);
        }
        result.map(|_| ())
    }

    fn validate_ref(
        &self,
        sch: SchemaIndex,
        kw: &'static str,
        uneval: &mut Uneval<'_>,
    ) -> Result<(), ValidationError> {
        if let Err(ref_err) = self.validate_self(sch, kw.into(), uneval) {
            let mut err = self.error(
                kw,
                ErrorKind::Reference {
                    url: self.schemas.get(sch).loc.clone(),
                },
            );
            if let ErrorKind::Group = ref_err.kind {
                err.causes = ref_err.causes;
            } else {
                err.causes.push(ref_err);
            }
            return Err(err);
        }
        Ok(())
    }

    fn resolve_recursive_anchor(&self) -> Option<SchemaIndex> {
        let mut scope = &self.scope;
        let mut sch = None;
        loop {
            let scope_sch = self.schemas.get(scope.sch);
            let base_sch = self.schemas.get(scope_sch.resource);
            if base_sch.recursive_anchor {
                sch.replace(scope.sch);
            }
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                return sch;
            }
        }
    }

    fn resolve_dynamic_anchor(&self, name: &String) -> Option<SchemaIndex> {
        let mut scope = &self.scope;
        let mut sch = None;
        loop {
            let scope_sch = self.schemas.get(scope.sch);
            let base_sch = self.schemas.get(scope_sch.resource);
            debug_assert_eq!(base_sch.idx, base_sch.resource);
            if let Some(dsch) = base_sch.dynamic_anchors.get(name) {
                sch.replace(*dsch);
            }
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                return sch;
            }
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
                if got.is_empty() {
                    write!(
                        f,
                        "maximum {want} items required to match contains schema, but found none",
                    )
                } else {
                    write!(
                        f,
                        "maximum {want} items required to match contains schema, but found {} items at {}",
                        got.len(),
                        join_iter(got, ", ")
                    )
                }
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
