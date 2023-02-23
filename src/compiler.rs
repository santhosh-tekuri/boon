use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt::Display;

use regex::Regex;
use serde_json::{Map, Value};
use url::Url;

use crate::content::{DECODERS, MEDIA_TYPES};
use crate::draft::{DRAFT2019, DRAFT2020, DRAFT4, DRAFT6, DRAFT7};
use crate::formats::FORMATS;
use crate::root::Root;
use crate::roots::Roots;
use crate::util::*;
use crate::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Draft {
    V4,
    V6,
    V7,
    V2019_09,
    V2020_12,
}

impl Draft {
    /// get [`Draft`] for given `url`
    ///
    /// # Arguments
    ///
    /// * `url` - accepts both `http` and `https` and ignores any fragments in url
    ///
    /// # Examples
    ///
    /// ```
    /// # use boon::*;
    /// assert_eq!(Draft::from_url("https://json-schema.org/draft/2020-12/schema"), Some(Draft::V2020_12));
    /// assert_eq!(Draft::from_url("http://json-schema.org/draft-07/schema#"), Some(Draft::V7));
    /// ```
    pub fn from_url(url: &str) -> Option<Draft> {
        match crate::draft::Draft::from_url(url) {
            Some(draft) => match draft.version {
                4 => Some(Draft::V4),
                6 => Some(Draft::V6),
                7 => Some(Draft::V7),
                2019 => Some(Draft::V2019_09),
                2020 => Some(Draft::V2020_12),
                _ => None,
            },
            None => None,
        }
    }

    pub(crate) fn internal(&self) -> &'static crate::draft::Draft {
        match self {
            Draft::V4 => &DRAFT4,
            Draft::V6 => &DRAFT6,
            Draft::V7 => &DRAFT7,
            Draft::V2019_09 => &DRAFT2019,
            Draft::V2020_12 => &DRAFT2020,
        }
    }
}

// returns latest draft supported
impl Default for Draft {
    fn default() -> Self {
        Draft::V2020_12
    }
}

#[derive(Default)]
pub struct Compiler {
    roots: Roots,
    assert_format: bool,
    assert_content: bool,
    formats: HashMap<&'static str, Format>,
    decoders: HashMap<&'static str, Decoder>,
    media_types: HashMap<&'static str, MediaType>,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default_draft(&mut self, d: Draft) {
        self.roots.default_draft = d.internal()
    }

    pub fn enable_format_assertions(&mut self) {
        self.assert_format = true;
    }

    pub fn enable_content_assertions(&mut self) {
        self.assert_content = true;
    }

    pub fn register_url_loader(&mut self, scheme: &'static str, url_loader: Box<dyn UrlLoader>) {
        self.roots.loader.register(scheme, url_loader);
    }

    pub fn register_format(&mut self, name: &'static str, format: Format) {
        self.formats.insert(name, format);
    }

    pub fn register_decoder(&mut self, content_encoding: &'static str, decoder: Decoder) {
        self.decoders.insert(content_encoding, decoder);
    }

    pub fn register_media_type(&mut self, media_type: &'static str, validator: MediaType) {
        self.media_types.insert(media_type, validator);
    }

    pub fn add_resource(&mut self, url: &str, json: Value) -> Result<bool, CompileError> {
        let url = Url::parse(url).map_err(|e| CompileError::LoadUrlError {
            url: url.to_owned(),
            src: e.into(),
        })?;
        self.roots.or_insert(url, json)
    }

    pub fn compile(
        &mut self,
        mut loc: String,
        target: &mut Schemas,
    ) -> Result<SchemaIndex, CompileError> {
        if loc.rfind('#').is_none() {
            loc.push('#');
        }

        let mut queue = VecDeque::new();
        let index = target.enqueue(&mut queue, loc);
        if queue.is_empty() {
            // already got compiled
            return Ok(index);
        }

        let mut sch_index = None;
        while let Some(loc) = queue.front() {
            let (url, ptr) = split(loc);
            let url = Url::parse(url).map_err(|e| CompileError::LoadUrlError {
                url: url.to_owned(),
                src: e.into(),
            })?;
            self.roots.or_load(url.clone())?;
            let root = self.roots.get(&url).unwrap();
            let v = root
                .lookup_ptr(ptr)
                .map_err(|_| CompileError::InvalidJsonPointer(loc.clone()))?;
            let Some(v) = v else {
                return Err(CompileError::JsonPointerNotFound(loc.to_owned()));
            };

            let sch = self.compile_value(target, v, loc.to_owned(), root, &mut queue)?;
            let loc = queue
                .pop_front()
                .ok_or(CompileError::Bug("queue must be non-empty".into()))?;
            let index = target.insert(loc, sch);
            sch_index = sch_index.or(Some(index));
        }
        sch_index.ok_or(CompileError::Bug("schema_index must exist".into()))
    }

    fn compile_value(
        &self,
        schemas: &Schemas,
        v: &Value,
        loc: String,
        root: &Root,
        queue: &mut VecDeque<String>,
    ) -> Result<Schema, CompileError> {
        let mut s = Schema::new(loc.clone());
        s.draft_version = root.draft.version;

        // we know it is already in queue, we just want to get its index
        s.idx = schemas.enqueue(queue, loc.to_owned());
        s.resource = {
            let (_, ptr) = split(&loc);
            let base = root.base_url(ptr);
            let base_loc = root.resolve(base.as_str())?;
            schemas.enqueue(queue, base_loc)
        };

        // enqueue dynamicAnchors for compilation
        if s.idx == s.resource && root.draft.version >= 2020 {
            let (url, ptr) = split(&loc);
            if let Some(res) = root.resource(ptr) {
                for danchor in &res.dynamic_anchors {
                    let danchor_ptr = res.anchors.get(danchor).unwrap();
                    let danchor_sch = schemas.enqueue(queue, format!("{url}#{danchor_ptr}"));
                    s.dynamic_anchors.insert(danchor.to_owned(), danchor_sch);
                }
            }
        }

        match v {
            Value::Object(obj) => {
                let mut h = Helper {
                    obj,
                    loc: &loc,
                    schemas,
                    queue,
                    root,
                };
                self.compile_obj(&mut s, obj, &mut h)?;
            }
            Value::Bool(b) => s.boolean = Some(*b),
            _ => {}
        }
        Ok(s)
    }

    fn compile_obj(
        &self,
        s: &mut Schema,
        obj: &Map<String, Value>,
        h: &mut Helper,
    ) -> Result<(), CompileError> {
        self.compile_draft4(s, obj, h)?;
        if h.draft_version() >= 6 {
            self.compile_draft6(s, h)?;
        }
        if h.draft_version() >= 7 {
            self.compile_draft7(s, h)?;
        }
        if h.draft_version() >= 2019 {
            self.compile_draft2019(s, h)?;
        }
        if h.draft_version() >= 2020 {
            self.compile_draft2020(s, h)?;
        }
        Ok(())
    }

    fn compile_draft4(
        &self,
        s: &mut Schema,
        obj: &Map<String, Value>,
        h: &mut Helper,
    ) -> Result<(), CompileError> {
        if h.has_vocab("core") {
            s.ref_ = h.enqueue_ref("$ref")?;
            if s.ref_.is_some() && h.draft_version() < 2019 {
                // All other properties in a "$ref" object MUST be ignored
                return Ok(());
            }
        }

        if h.has_vocab("applicator") {
            s.all_of = h.enqueue_arr("allOf");
            s.any_of = h.enqueue_arr("anyOf");
            s.one_of = h.enqueue_arr("oneOf");
            s.not = h.enqueue_prop("not");

            if h.draft_version() < 2020 {
                match obj.get("items") {
                    Some(Value::Array(_)) => {
                        s.items = Some(Items::SchemaRefs(h.enqueue_arr("items")));
                        s.additional_items = h.enquue_additional("additionalItems");
                    }
                    _ => s.items = h.enqueue_prop("items").map(Items::SchemaRef),
                }
            }

            s.properties = h.enqueue_map("properties");
            s.pattern_properties = {
                let mut v = vec![];
                if let Some(Value::Object(obj)) = obj.get("patternProperties") {
                    for pname in obj.keys() {
                        let regex = Regex::new(pname).map_err(|_| CompileError::InvalidRegex {
                            url: format!("{}/patternProperties", h.loc),
                            regex: pname.clone(),
                        })?;
                        let sch = h.enqueue_path(format!("patternProperties/{}", escape(pname)));
                        v.push((regex, sch));
                    }
                }
                v
            };

            s.additional_properties = h.enquue_additional("additionalProperties");

            if let Some(Value::Object(deps)) = obj.get("dependencies") {
                s.dependencies = deps
                    .iter()
                    .filter_map(|(k, v)| {
                        let v = match v {
                            Value::Array(_) => Some(Dependency::Props(to_strings(v))),
                            _ => Some(Dependency::SchemaRef(
                                h.enqueue_path(format!("dependencies/{}", escape(k))),
                            )),
                        };
                        v.map(|v| (k.clone(), v))
                    })
                    .collect();
            }
        }

        if h.has_vocab("validation") {
            match h.value("type") {
                Some(Value::String(t)) => s.types.extend(Type::from_str(t)),
                Some(Value::Array(tt)) => {
                    s.types.extend(tt.iter().filter_map(|t| {
                        if let Value::String(t) = t {
                            Type::from_str(t)
                        } else {
                            None
                        }
                    }));
                }
                _ => {}
            }

            if let Some(Value::Array(e)) = h.value("enum") {
                s.enum_ = e.clone();
            }

            s.multiple_of = h.num("multipleOf");

            s.maximum = h.num("maximum");
            if let Some(Value::Bool(exclusive)) = h.value("exclusiveMaximum") {
                if *exclusive {
                    s.exclusive_maximum = s.maximum.take();
                }
            } else {
                s.exclusive_maximum = h.num("exclusiveMaximum");
            }

            s.minimum = h.num("minimum");
            if let Some(Value::Bool(exclusive)) = h.value("exclusiveMinimum") {
                if *exclusive {
                    s.exclusive_minimum = s.minimum.take();
                }
            } else {
                s.exclusive_minimum = h.num("exclusiveMinimum");
            }

            s.max_length = h.usize("maxLength");
            s.min_length = h.usize("minLength");

            if let Some(Value::String(p)) = h.value("pattern") {
                s.pattern = Some(Regex::new(p).map_err(|e| CompileError::Bug(e.into()))?);
            }

            s.max_items = h.usize("maxItems");
            s.min_items = h.usize("minItems");
            s.unique_items = h.bool("uniqueItems");

            s.max_properties = h.usize("maxProperties");
            s.min_properties = h.usize("minProperties");

            if let Some(req) = h.value("required") {
                s.required = to_strings(req);
            }
        }

        // format --
        if self.assert_format
            || h.has_vocab(match h.draft_version().cmp(&2019) {
                Ordering::Less => "core",
                Ordering::Equal => "format",
                Ordering::Greater => "format-assertion",
            })
        {
            if let Some(Value::String(format)) = h.value("format") {
                let func = self
                    .formats
                    .get(format.as_str())
                    .or_else(|| FORMATS.get(format.as_str()));
                if let Some(func) = func {
                    s.format = Some((format.to_owned(), *func));
                }
            }
        }

        Ok(())
    }

    fn compile_draft6(&self, s: &mut Schema, h: &mut Helper) -> Result<(), CompileError> {
        if h.has_vocab("applicator") {
            s.contains = h.enqueue_prop("contains");
            s.property_names = h.enqueue_prop("propertyNames");
        }

        if h.has_vocab("validation") {
            s.constant = h.value("const").cloned();
        }

        Ok(())
    }

    fn compile_draft7(&self, s: &mut Schema, h: &mut Helper) -> Result<(), CompileError> {
        if h.has_vocab("applicator") {
            s.if_ = h.enqueue_prop("if");
            if s.if_.is_some() {
                s.then = h.enqueue_prop("then");
                s.else_ = h.enqueue_prop("else");
            }
        }

        if self.assert_content {
            if let Some(Value::String(encoding)) = h.value("contentEncoding") {
                let func = self
                    .decoders
                    .get(encoding.as_str())
                    .or_else(|| DECODERS.get(encoding.as_str()));
                if let Some(func) = func {
                    s.content_encoding = Some((encoding.to_owned(), *func));
                }
            }

            if let Some(Value::String(media_type)) = h.value("contentMediaType") {
                let func = self
                    .media_types
                    .get(media_type.as_str())
                    .or_else(|| MEDIA_TYPES.get(media_type.as_str()));
                if let Some(func) = func {
                    s.content_media_type = Some((media_type.to_owned(), *func));
                }
            }
        }

        Ok(())
    }

    fn compile_draft2019(&self, s: &mut Schema, h: &mut Helper) -> Result<(), CompileError> {
        if h.has_vocab("core") {
            s.recursive_ref = h.enqueue_ref("$recursiveRef")?;
            s.recursive_anchor = h.bool("$recursiveAnchor");
        }

        if h.has_vocab("validation") {
            if s.contains.is_some() {
                s.max_contains = h.usize("maxContains");
                s.min_contains = h.usize("minContains");
            }

            if let Some(Value::Object(dep_req)) = h.value("dependentRequired") {
                for (pname, pvalue) in dep_req {
                    s.dependent_required
                        .insert(pname.clone(), to_strings(pvalue));
                }
            }
        }

        if h.has_vocab("applicator") {
            s.dependent_schemas = h.enqueue_map("dependentSchemas");
        }

        if h.has_vocab(match h.draft_version() {
            2019 => "applicator",
            _ => "unevaluated",
        }) {
            s.unevaluated_items = h.enqueue_prop("unevaluatedItems");
            s.unevaluated_properties = h.enqueue_prop("unevaluatedProperties");
        }

        Ok(())
    }

    fn compile_draft2020(&self, s: &mut Schema, h: &mut Helper) -> Result<(), CompileError> {
        if h.has_vocab("core") {
            s.dynamic_ref = h.enqueue_ref("$dynamicRef")?;
            if let Some(Value::String(anchor)) = h.value("$dynamicAnchor") {
                s.dynamic_anchor = Some(anchor.to_owned());
            }
        }

        if h.has_vocab("applicator") {
            s.prefix_items = h.enqueue_arr("prefixItems");
            s.items2020 = h.enqueue_prop("items");
        }

        Ok(())
    }
}

struct Helper<'a, 'b, 'c, 'd, 'e> {
    obj: &'a Map<String, Value>,
    loc: &'c str,
    schemas: &'d Schemas,
    queue: &'b mut VecDeque<String>,
    root: &'e Root,
}

impl<'a, 'b, 'c, 'd, 'e> Helper<'a, 'b, 'c, 'd, 'e> {
    fn draft_version(&self) -> usize {
        self.root.draft.version
    }

    fn has_vocab(&self, name: &str) -> bool {
        self.root.has_vocab(name)
    }

    fn value(&self, pname: &str) -> Option<&Value> {
        self.obj.get(pname)
    }

    fn bool(&self, pname: &str) -> bool {
        matches!(self.obj.get(pname), Some(Value::Bool(true)))
    }

    fn usize(&self, pname: &str) -> Option<usize> {
        if let Some(Value::Number(n)) = self.obj.get(pname) {
            if n.is_u64() {
                n.as_u64().map(|n| n as usize)
            } else {
                n.as_f64()
                    .filter(|n| n.is_sign_positive() && n.fract() == 0.0)
                    .map(|n| n as usize)
            }
        } else {
            None
        }
    }

    fn num(&self, pname: &str) -> Option<Number> {
        if let Some(Value::Number(n)) = self.obj.get(pname) {
            Some(n.clone())
        } else {
            None
        }
    }

    fn enqueue_path(&mut self, path: String) -> SchemaIndex {
        let loc = format!("{}/{path}", self.loc);
        self.schemas.enqueue(self.queue, loc)
    }

    fn enqueue_prop(&mut self, pname: &str) -> Option<SchemaIndex> {
        if self.obj.contains_key(pname) {
            let loc = format!("{}/{}", self.loc, escape(pname));
            Some(self.schemas.enqueue(self.queue, loc))
        } else {
            None
        }
    }

    fn enqueue_arr(&mut self, pname: &str) -> Vec<SchemaIndex> {
        if let Some(Value::Array(arr)) = self.obj.get(pname) {
            (0..arr.len())
                .map(|i| {
                    let loc = format!("{}/{pname}/{i}", self.loc);
                    self.schemas.enqueue(self.queue, loc)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn enqueue_map(&mut self, pname: &str) -> HashMap<String, SchemaIndex> {
        if let Some(Value::Object(obj)) = self.obj.get(pname) {
            obj.keys()
                .map(|k| {
                    let loc = format!("{}/{pname}/{}", self.loc, escape(k));
                    (k.clone(), self.schemas.enqueue(self.queue, loc))
                })
                .collect()
        } else {
            HashMap::new()
        }
    }

    fn enqueue_ref(&mut self, pname: &str) -> Result<Option<SchemaIndex>, CompileError> {
        if let Some(Value::String(ref_)) = self.obj.get(pname) {
            let (_, ptr) = split(self.loc);
            let abs_ref =
                self.root
                    .base_url(ptr)
                    .join(ref_)
                    .map_err(|e| CompileError::ParseUrlError {
                        url: ref_.clone(),
                        src: e.into(),
                    })?;
            let resolved_ref = self.root.resolve(abs_ref.as_str())?;
            Ok(Some(self.schemas.enqueue(self.queue, resolved_ref)))
        } else {
            Ok(None)
        }
    }

    fn enquue_additional(&mut self, pname: &str) -> Option<Additional> {
        if let Some(Value::Bool(b)) = self.obj.get(pname) {
            Some(Additional::Bool(*b))
        } else {
            self.enqueue_prop(pname).map(Additional::SchemaRef)
        }
    }
}

#[derive(Debug)]
pub enum CompileError {
    ParseUrlError {
        url: String,
        src: Box<dyn Error>,
    },
    LoadUrlError {
        url: String,
        src: Box<dyn Error>,
    },
    UnsupportedUrlScheme {
        url: String,
    },
    InvalidMetaSchema {
        url: String,
    },
    MetaSchemaCycle {
        url: String,
    },
    NotValid(ValidationError),
    InvalidId {
        loc: String,
    },
    InvalidAnchor {
        loc: String,
    },
    DuplicateId {
        url: String,
        id: String,
    },
    DuplicateAnchor {
        anchor: String,
        url: String,
        ptr1: String,
        ptr2: String,
    },
    InvalidJsonPointer(String),
    JsonPointerNotFound(String),
    AnchorNotFound {
        schema_url: String,
        anchor_url: String,
    },
    UnsupprtedVocabulary {
        url: String,
        vocabulary: String,
    },
    InvalidRegex {
        url: String,
        regex: String,
    },
    Bug(Box<dyn Error>),
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LoadUrlError { src, .. } => Some(src.as_ref()),
            _ => None,
        }
    }
}

impl Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseUrlError { url, src } => {
                if f.alternate() {
                    write!(f, "error parsing url {url}: {src}")
                } else {
                    write!(f, "error parsing {url}")
                }
            }
            Self::LoadUrlError { url, src } => {
                if f.alternate() {
                    write!(f, "error loading {url}: {src}")
                } else {
                    write!(f, "error loading {url}")
                }
            }
            Self::UnsupportedUrlScheme { url } => write!(f, "unsupported scheme in {url}"),
            Self::InvalidMetaSchema { url } => write!(f, "invalid $schema in {url}"),
            Self::MetaSchemaCycle { url } => {
                write!(f, "cycle in resolving $schema in {url}")
            }
            Self::NotValid(ve) => {
                if f.alternate() {
                    write!(f, "not valid against metaschema: {ve:#}")
                } else {
                    write!(f, "not valid against metaschema")
                }
            }
            Self::InvalidId { loc } => write!(f, "invalid $id at {loc}"),
            Self::InvalidAnchor { loc } => write!(f, "invalid $anchor at {loc}"),
            Self::DuplicateId { url, id } => write!(f, "duplicate $id {id} in {url}"),
            Self::DuplicateAnchor {
                anchor,
                url,
                ptr1,
                ptr2,
            } => {
                write!(
                    f,
                    "duplicate anchor {anchor:?} in {url} at {ptr1:?} and {ptr2:?}"
                )
            }
            Self::InvalidJsonPointer(loc) => write!(f, "invalid json-pointer {loc}"),
            Self::JsonPointerNotFound(loc) => write!(f, "json-pointer in {loc} not found"),
            Self::AnchorNotFound {
                schema_url,
                anchor_url,
            } => {
                write!(
                    f,
                    "anchor in {anchor_url} is not found in schema {schema_url}"
                )
            }
            Self::UnsupprtedVocabulary { url, vocabulary } => {
                write!(f, "unsupported vocabulary {vocabulary} in {url}")
            }
            Self::InvalidRegex { url, regex } => {
                write!(f, "invalid regex {} at {}", quote(regex), url)
            }
            Self::Bug(src) => {
                write!(
                    f,
                    "encountered bug in jsonschema compiler. please report: {src}"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiler() {
        let sch: Value = serde_json::from_str(r#"{"type":"string"}"#).unwrap();
        let mut c = Compiler::default();
        let url = Url::parse("http://a.com/schema.json").unwrap();
        c.roots.or_insert(url.clone(), sch).unwrap();
        let loc = format!("{url}#");
        let mut schemas = Schemas::default();
        let sch_index = c.compile(loc, &mut schemas).unwrap();
        let inst: Value = Value::String("xx".into());
        schemas.validate(&inst, sch_index).unwrap();
    }

    #[test]
    fn test_debug() {
        run_single(
            Draft::V6,
            r##"
            {"type": "integer"}            
            "##,
            r##"
            1.0
            "##,
            true,
        );
    }

    fn run_single(draft: Draft, schema: &str, data: &str, valid: bool) {
        let schema: Value = serde_json::from_str(schema).unwrap();
        let data: Value = serde_json::from_str(data).unwrap();

        let url = "http://testsuite.com/schema.json";
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        compiler.set_default_draft(draft);
        compiler.add_resource(url, schema).unwrap();
        let sch_index = compiler.compile(url.into(), &mut schemas).unwrap();
        let result = schemas.validate(&data, sch_index);
        if let Err(e) = &result {
            println!("validation failed: {e:#}");
        }
        assert_eq!(result.is_ok(), valid);
    }
}
