use std::cell::BorrowMutError;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;

use regex::Regex;
use serde_json::Value;
use url::Url;

use crate::root::Root;
use crate::roots::Roots;
use crate::util::*;
use crate::*;

#[derive(Default)]
struct Compiler {
    roots: Roots,
    decoders: HashMap<String, Decoder>,
    media_types: HashMap<String, MediaType>,
}

impl Compiler {
    fn compile(&mut self, target: &mut Schemas, loc: String) -> Result<(), CompileError> {
        let mut queue = vec![];
        queue.push(loc);
        while let Some(loc) = queue.pop() {
            let (url, ptr) = split(&loc);
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
                return Err(CompileError::NotFound(loc));
            };

            let sch = self.compile_one(target, v, loc.clone(), root, &mut queue)?;
            target.insert(loc, sch);
        }
        Ok(())
    }

    fn compile_one(
        &self,
        schemas: &Schemas,
        v: &Value,
        loc: String,
        root: &Root,
        queue: &mut Vec<String>,
    ) -> Result<Schema, CompileError> {
        let mut s = Schema::new(loc.clone());
        let Value::Object(obj) = v else {
            return Ok(s);
        };

        // helpers --
        let load_usize = |pname| {
            if let Some(Value::Number(n)) = obj.get(pname) {
                n.as_u64().map(|n| n as usize)
            } else {
                None
            }
        };
        let load_num = |pname| {
            if let Some(Value::Number(n)) = obj.get(pname) {
                Some(n.clone())
            } else {
                None
            }
        };
        let to_strings = |v: &Value| {
            if let Value::Array(a) = v {
                a.iter()
                    .filter_map(|t| {
                        if let Value::String(t) = t {
                            Some(t.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            }
        };
        let load_schema = |pname, queue: &mut Vec<String>| {
            if obj.contains_key(pname) {
                Some(schemas.enqueue(queue, format!("{loc}/{}", escape(pname))))
            } else {
                None
            }
        };
        let load_schema_arr = |pname, queue: &mut Vec<String>| {
            if let Some(Value::Array(arr)) = obj.get(pname) {
                (0..arr.len())
                    .map(|i| schemas.enqueue(queue, format!("{loc}/{pname}/{i}")))
                    .collect()
            } else {
                Vec::new()
            }
        };
        let load_schema_map = |pname, queue: &mut Vec<String>| {
            if let Some(Value::Object(obj)) = obj.get(pname) {
                obj.keys()
                    .map(|k| {
                        (
                            k.clone(),
                            schemas.enqueue(queue, format!("{loc}/{pname}/{}", escape(k))),
                        )
                    })
                    .collect()
            } else {
                HashMap::new()
            }
        };

        // draft4 --
        if let Some(t) = obj.get("type") {
            match t {
                Value::String(t) => s.types.extend(Type::from_str(t)),
                Value::Array(tt) => {
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
        }

        if let Some(Value::Array(e)) = obj.get("enum") {
            s.enum_ = e.clone();
        }

        s.minimum = load_num("minimum");
        if let Some(Value::Bool(exclusive)) = obj.get("exclusiveMinimum") {
            if *exclusive {
                (s.minimum, s.exclusive_minimum) = (None, s.minimum);
            }
        } else {
            s.exclusive_minimum = load_num("exclusiveMinimum");
        }

        s.maximum = load_num("maximum");
        if let Some(Value::Bool(exclusive)) = obj.get("exclusiveMaximum") {
            if *exclusive {
                (s.maximum, s.exclusive_maximum) = (None, s.maximum);
            }
        } else {
            s.exclusive_maximum = load_num("exclusiveMaximum");
        }

        s.multiple_of = load_num("multipleOf");

        s.min_properties = load_usize("minProperties");
        s.max_properties = load_usize("maxProperties");

        if let Some(req) = obj.get("required") {
            s.required = to_strings(req);
        }

        s.min_items = load_usize("minItems");
        s.max_items = load_usize("maxItems");
        if let Some(Value::Bool(unique)) = obj.get("uniqueItems") {
            s.unique_items = *unique;
        }

        s.min_length = load_usize("minLength");
        s.max_length = load_usize("maxlength");

        if let Some(Value::String(p)) = obj.get("pattern") {
            s.pattern = Some(Regex::new(p).map_err(|e| CompileError::Bug(e.into()))?);
        }

        s.not = load_schema("not", queue);
        s.all_of = load_schema_arr("allOf", queue);
        s.any_of = load_schema_arr("anyOf", queue);
        s.one_of = load_schema_arr("oneOf", queue);
        s.properties = load_schema_map("properties", queue);

        if root.draft.version < 2020 {
            match obj.get("items") {
                Some(Value::Array(_)) => {
                    s.items = Some(Items::SchemaRefs(load_schema_arr("items", queue)));
                    s.additional_properties = {
                        if let Some(Value::Bool(b)) = obj.get("additionalProperties") {
                            Some(AdditionalProperties::Bool(*b))
                        } else {
                            load_schema("additionalProperties", queue)
                                .map(AdditionalProperties::SchemaRef)
                        }
                    };
                }
                _ => s.items = load_schema("items", queue).map(Items::SchemaRef),
            }
        }

        if let Some(Value::Object(obj)) = obj.get("dependencies") {
            s.dependencies = obj
                .iter()
                .filter_map(|(k, v)| {
                    let v = match v {
                        Value::Array(_) => Some(Dependency::Props(to_strings(v))),
                        Value::Object(_) => Some(Dependency::SchemaRef(
                            schemas.enqueue(queue, format!("{loc}/dependencies/{}", escape(k))),
                        )),
                        _ => None,
                    };
                    v.map(|v| (k.clone(), v))
                })
                .collect();
        }

        // draft6 --
        if root.draft.version >= 6 {
            s.property_names = load_schema("propertyNames", queue);
            s.contains = load_schema("contains", queue);
        }

        // draft7 --
        if root.draft.version >= 7 {
            s.if_ = load_schema("if", queue);
            s.then = load_schema("then", queue);
            s.else_ = load_schema("else", queue);
        }

        // draft2019 --
        if root.draft.version >= 2019 {
            s.min_contains = load_usize("minContains");
            s.max_contains = load_usize("maxContains");
            s.dependent_schemas = load_schema_map("dependentSchemas", queue);

            if let Some(Value::Object(deps)) = obj.get("dependentRequired") {
                for (pname, pvalue) in deps {
                    s.dependent_required
                        .insert(pname.clone(), to_strings(pvalue));
                }
            }
        }

        // draft2020 --
        if root.draft.version >= 2020 {
            s.prefix_items = load_schema("prefixItems", queue);
            s.items2020 = load_schema("items", queue);
        }

        Ok(s)
    }
}

#[derive(Debug)]
pub enum CompileError {
    LoadUrlError { url: String, src: Box<dyn Error> },
    UnsupportedUrl { url: String },
    InvalidMetaSchema { url: String },
    MetaSchemaCycle { url: String },
    InvalidId { loc: String },
    DuplicateId { url: String, id: String },
    InvalidJsonPointer(String),
    NotFound(String),
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
            Self::LoadUrlError { url, src } => {
                if f.alternate() {
                    write!(f, "error loading {url}: {src}")
                } else {
                    write!(f, "error loading {url}")
                }
            }
            Self::UnsupportedUrl { url } => write!(f, "loading {url} unsupported"),
            Self::InvalidMetaSchema { url } => write!(f, "invalid $schema in {url}"),
            Self::MetaSchemaCycle { url } => {
                write!(f, "cycle in resolving $schema in {url}")
            }
            Self::InvalidId { loc } => write!(f, "invalid $id at {loc}"),
            Self::DuplicateId { url, id } => write!(f, "duplicate $id {id} in {url}"),
            Self::InvalidJsonPointer(loc) => write!(f, "invalid json pointer {loc}"),
            Self::NotFound(loc) => write!(f, "{loc} not found"),
            Self::Bug(src) => {
                write!(
                    f,
                    "encountered bug in jsonschema compiler. please report: {src}"
                )
            }
        }
    }
}

impl From<BorrowMutError> for CompileError {
    fn from(value: BorrowMutError) -> Self {
        Self::Bug(value.into())
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
        c.compile(&mut schemas, loc.clone()).unwrap();
        let sch = schemas.get(&loc).unwrap();
        println!("{:?}", sch.types);
        println!("{:?}", schemas.map);
        let inst: Value = Value::String("xx".into());
        sch.validate(&inst).unwrap();
    }
}
