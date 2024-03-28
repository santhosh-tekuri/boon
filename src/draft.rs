use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
    usize,
};

use once_cell::sync::Lazy;
use serde_json::{Map, Value};
use url::Url;

use crate::{compiler::*, root::Resource, util::*, SchemaIndex, Schemas};

const POS_SELF: u8 = 1 << 0;
const POS_PROP: u8 = 1 << 1;
const POS_ITEM: u8 = 1 << 2;

pub(crate) static DRAFT4: Lazy<Draft> = Lazy::new(|| Draft {
    version: 4,
    id: "id",
    subschemas: HashMap::from([
        // type agnostic
        ("definitions", POS_PROP),
        ("not", POS_SELF),
        ("allOf", POS_ITEM),
        ("anyOf", POS_ITEM),
        ("oneOf", POS_ITEM),
        // object
        ("properties", POS_PROP),
        ("additionalProperties", POS_SELF),
        ("patternProperties", POS_PROP),
        // array
        ("items", POS_SELF | POS_ITEM),
        ("additionalItems", POS_SELF),
        ("dependencies", POS_PROP),
    ]),
    vocab_prefix: "",
    all_vocabs: vec![],
    default_vocabs: vec![],
});

pub(crate) static DRAFT6: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT4.subschemas.clone();
    subschemas.extend([("propertyNames", POS_SELF), ("contains", POS_SELF)]);
    Draft {
        version: 6,
        id: "$id",
        subschemas,
        vocab_prefix: "",
        all_vocabs: vec![],
        default_vocabs: vec![],
    }
});

pub(crate) static DRAFT7: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT6.subschemas.clone();
    subschemas.extend([("if", POS_SELF), ("then", POS_SELF), ("else", POS_SELF)]);
    Draft {
        version: 7,
        id: "$id",
        subschemas,
        vocab_prefix: "",
        all_vocabs: vec![],
        default_vocabs: vec![],
    }
});

pub(crate) static DRAFT2019: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT7.subschemas.clone();
    subschemas.extend([
        ("$defs", POS_PROP),
        ("dependentSchemas", POS_PROP),
        ("unevaluatedProperties", POS_SELF),
        ("unevaluatedItems", POS_SELF),
        ("contentSchema", POS_SELF),
    ]);
    Draft {
        version: 2019,
        id: "$id",
        subschemas,
        vocab_prefix: "https://json-schema.org/draft/2019-09/vocab/",
        all_vocabs: vec![
            "core",
            "applicator",
            "validation",
            "meta-data",
            "format",
            "content",
        ],
        default_vocabs: vec!["core", "applicator", "validation"],
    }
});

pub(crate) static DRAFT2020: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT2019.subschemas.clone();
    subschemas.extend([("prefixItems", POS_ITEM)]);
    Draft {
        version: 2020,
        id: "$id",
        subschemas,
        vocab_prefix: "https://json-schema.org/draft/2020-12/vocab/",
        all_vocabs: vec![
            "core",
            "applicator",
            "unevaluated",
            "validation",
            "meta-data",
            "format-annotation",
            "format-assertion",
            "content",
        ],
        default_vocabs: vec!["core", "applicator", "unevaluated", "validation"],
    }
});

pub(crate) static STD_METASCHEMAS: Lazy<Schemas> =
    Lazy::new(|| load_std_metaschemas().expect("std metaschemas must be compilable"));

pub(crate) fn latest() -> &'static Draft {
    crate::Draft::default().internal()
}

// --

pub(crate) struct Draft {
    pub(crate) version: usize,
    id: &'static str,
    subschemas: HashMap<&'static str, u8>,
    pub(crate) vocab_prefix: &'static str,
    pub(crate) all_vocabs: Vec<&'static str>,
    pub(crate) default_vocabs: Vec<&'static str>,
}

impl Draft {
    pub(crate) fn from_url(url: &str) -> Option<&'static Draft> {
        let (mut url, fragment) = split(url);
        if !fragment.as_str().is_empty() {
            return None;
        }
        if let Some(s) = url.strip_prefix("http://") {
            url = s;
        }
        if let Some(s) = url.strip_prefix("https://") {
            url = s;
        }
        match url {
            "json-schema.org/schema" => Some(latest()),
            "json-schema.org/draft/2020-12/schema" => Some(&DRAFT2020),
            "json-schema.org/draft/2019-09/schema" => Some(&DRAFT2019),
            "json-schema.org/draft-07/schema" => Some(&DRAFT7),
            "json-schema.org/draft-06/schema" => Some(&DRAFT6),
            "json-schema.org/draft-04/schema" => Some(&DRAFT4),
            _ => None,
        }
    }

    pub(crate) fn get_schema(&self) -> Option<SchemaIndex> {
        let loc = match self.version {
            2020 => Some("https://json-schema.org/draft/2020-12/schema#"),
            2019 => Some("https://json-schema.org/draft/2019-09/schema#"),
            7 => Some("http://json-schema.org/draft-07/schema#"),
            6 => Some("http://json-schema.org/draft-06/schema#"),
            4 => Some("http://json-schema.org/draft-04/schema#"),
            _ => None,
        };
        loc.and_then(|loc| STD_METASCHEMAS.get_by_loc(loc))
            .map(|s| s.idx)
    }

    fn get_id<'a>(&self, obj: &'a Map<String, Value>) -> Option<&'a Value> {
        if self.version < 2019 {
            if obj.contains_key("$ref") {
                None // All other properties in a "$ref" object MUST be ignored
            } else {
                match obj.get(self.id) {
                    Some(Value::String(id)) if id.starts_with('#') => None, // anchor only
                    id => id,
                }
            }
        } else {
            obj.get(self.id)
        }
    }

    // collects anchors/dynamic_achors from `sch` into `res`.
    // note this does not collect from subschemas in sch.
    pub(crate) fn collect_anchors(
        &self,
        sch: &Value,
        root_ptr: &str,
        res: &mut Resource,
        root_url: &Url,
    ) -> Result<(), CompileError> {
        let Value::Object(obj) = sch else {
            return Ok(());
        };

        let mut add_anchor = |anchor: String| match res.anchors.entry(anchor) {
            Entry::Occupied(entry) => {
                if entry.get() == root_ptr {
                    // anchor with same root_ptr already exists
                    return Ok(());
                }
                return Err(CompileError::DuplicateAnchor {
                    url: root_url.as_str().to_owned(),
                    anchor: entry.key().to_owned(),
                    ptr1: entry.get().to_owned(),
                    ptr2: root_ptr.to_owned(),
                });
            }
            entry => {
                entry.or_insert(root_ptr.to_owned());
                Ok(())
            }
        };

        if self.version < 2019 {
            if obj.contains_key("$ref") {
                return Ok(()); // All other properties in a "$ref" object MUST be ignored
            }
            // anchor is specified in id
            if let Some(Value::String(id)) = obj.get(self.id) {
                let (_, frag) = split(id);
                let Ok(anchor) = frag.to_anchor() else {
                    let mut url = root_url.clone();
                    url.set_fragment(Some(root_ptr));
                    return Err(CompileError::ParseAnchorError { loc: url.into() });
                };
                if let Some(anchor) = anchor {
                    add_anchor(anchor.into())?;
                };
                return Ok(());
            }
        }
        if self.version >= 2019 {
            if let Some(Value::String(anchor)) = obj.get("$anchor") {
                add_anchor(anchor.into())?;
            }
        }
        if self.version >= 2020 {
            if let Some(Value::String(anchor)) = obj.get("$dynamicAnchor") {
                add_anchor(anchor.clone())?;
                res.dynamic_anchors.insert(anchor.clone());
            }
        }
        Ok(())
    }

    // error is json-ptr to invalid id
    pub(crate) fn collect_resources(
        &self,
        sch: &Value,
        base: &Url,       // base of json
        root_ptr: String, // ptr of json
        root_url: &Url,
        resources: &mut HashMap<String, Resource>,
    ) -> Result<(), CompileError> {
        if resources.contains_key(&root_ptr) {
            // resources are already collected
            return Ok(());
        }
        if let Value::Bool(_) = sch {
            if root_ptr.is_empty() {
                // root resource
                resources.insert(root_ptr.clone(), Resource::new(root_ptr, base.clone()));
            }
            return Ok(());
        }

        let Value::Object(obj) = sch else {
            return Ok(());
        };

        let id = self.get_id(obj);

        let mut base = base;
        let tmp;
        let res = if let Some(Value::String(id)) = id {
            let (id, _) = split(id);
            let Ok(id) = base.join(id) else {
                let mut url = base.clone();
                url.set_fragment(Some(&root_ptr));
                return Err(CompileError::ParseIdError { loc: url.into() });
            };
            tmp = id;
            base = &tmp;
            Some(Resource::new(root_ptr.clone(), base.clone()))
        } else if root_ptr.is_empty() {
            // root resource
            Some(Resource::new(root_ptr.clone(), base.clone()))
        } else {
            None
        };
        if let Some(res) = res {
            if let Some(dup) = resources.values_mut().find(|res| res.id == *base) {
                return Err(CompileError::DuplicateId {
                    url: root_url.to_string(),
                    id: base.to_string(),
                    ptr1: res.ptr.to_string(),
                    ptr2: dup.ptr.to_string(),
                });
            }
            resources.insert(root_ptr.clone(), res);
        }

        // collect anchors
        if let Some(res) = resources.values_mut().find(|res| res.id == *base) {
            self.collect_anchors(sch, &root_ptr, res, root_url)?;
        } else {
            debug_assert!(false, "base resource must exist");
        }

        for (&kw, &pos) in &self.subschemas {
            let Some(v) = obj.get(kw) else {
                continue;
            };
            if pos & POS_SELF != 0 {
                let ptr = format!("{root_ptr}/{kw}");
                self.collect_resources(v, base, ptr, root_url, resources)?;
            }
            if pos & POS_ITEM != 0 {
                if let Value::Array(arr) = v {
                    for (i, item) in arr.iter().enumerate() {
                        let ptr = format!("{root_ptr}/{kw}/{i}");
                        self.collect_resources(item, base, ptr, root_url, resources)?;
                    }
                }
            }
            if pos & POS_PROP != 0 {
                if let Value::Object(obj) = v {
                    for (pname, pvalue) in obj {
                        let ptr = format!("{root_ptr}/{kw}/{}", escape(pname));
                        self.collect_resources(pvalue, base, ptr, root_url, resources)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn is_subschema(&self, ptr: &str) -> bool {
        if ptr.is_empty() {
            return true;
        }

        fn split(ptr: &str) -> (&str, &str) {
            if let Some(i) = ptr[1..].find('/') {
                (&ptr[1..i + 1], &ptr[1 + i..])
            } else {
                (&ptr[1..], "")
            }
        }

        let (token, ptr) = split(ptr);

        if let Some(&pos) = self.subschemas.get(token) {
            if pos & POS_SELF != 0 && self.is_subschema(ptr) {
                return true;
            }
            if !ptr.is_empty() {
                if pos & POS_PROP != 0 {
                    let (_, ptr) = split(ptr);
                    if self.is_subschema(ptr) {
                        return true;
                    }
                }
                if pos & POS_ITEM != 0 {
                    let (tok, ptr) = split(ptr);
                    if usize::from_str(tok).is_ok() && self.is_subschema(ptr) {
                        return true;
                    }
                }
            }
        }

        false
    }
}

fn load_std_metaschemas() -> Result<Schemas, CompileError> {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_format_assertions();
    compiler.compile("https://json-schema.org/draft/2020-12/schema", &mut schemas)?;
    compiler.compile("https://json-schema.org/draft/2019-09/schema", &mut schemas)?;
    compiler.compile("http://json-schema.org/draft-07/schema", &mut schemas)?;
    compiler.compile("http://json-schema.org/draft-06/schema", &mut schemas)?;
    compiler.compile("http://json-schema.org/draft-04/schema", &mut schemas)?;
    Ok(schemas)
}

#[cfg(test)]
mod tests {
    use crate::{Compiler, Schemas};

    use super::*;

    #[test]
    fn test_meta() {
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        let v: Value = serde_json::from_str(include_str!("metaschemas/draft-04/schema")).unwrap();
        let url = "https://json-schema.org/draft-04/schema";
        compiler.add_resource(url, v).unwrap();
        compiler.compile(url, &mut schemas).unwrap();
    }

    #[test]
    fn test_from_url() {
        let tests = [
            ("http://json-schema.org/draft/2020-12/schema", Some(2020)), // http url
            ("https://json-schema.org/draft/2020-12/schema", Some(2020)), // https url
            ("https://json-schema.org/schema", Some(latest().version)),  // latest
            ("https://json-schema.org/draft-04/schema", Some(4)),
        ];
        for (url, version) in tests {
            let got = Draft::from_url(url).map(|d| d.version);
            assert_eq!(got, version, "for {url}");
        }
    }

    #[test]
    fn test_collect_ids() {
        let url = Url::parse("http://a.com/schema.json").unwrap();
        let json: Value = serde_json::from_str(
            r#"{
                "id": "http://a.com/schemas/schema.json",
                "definitions": {
                    "s1": { "id": "http://a.com/definitions/s1" },
                    "s2": {
                        "id": "../s2",
                        "items": [
                            { "id": "http://c.com/item" },
                            { "id": "http://d.com/item" }
                        ]
                    },
                    "s3": {
                        "definitions": {
                            "s1": {
                                "id": "s3",
                                "items": {
                                    "id": "http://b.com/item"
                                }
                            }
                        }
                    },
                    "s4": { "id": "http://e.com/def#abcd" }
                }
            }"#,
        )
        .unwrap();

        let want = {
            let mut m = HashMap::new();
            m.insert("", "http://a.com/schemas/schema.json"); // root with id
            m.insert("/definitions/s1", "http://a.com/definitions/s1");
            m.insert("/definitions/s2", "http://a.com/s2"); // relative id
            m.insert("/definitions/s3/definitions/s1", "http://a.com/schemas/s3");
            m.insert("/definitions/s3/definitions/s1/items", "http://b.com/item");
            m.insert("/definitions/s2/items/0", "http://c.com/item");
            m.insert("/definitions/s2/items/1", "http://d.com/item");
            m.insert("/definitions/s4", "http://e.com/def"); // id with fragments
            m
        };
        let mut got = HashMap::new();
        DRAFT4
            .collect_resources(&json, &url, String::new(), &url, &mut got)
            .unwrap();
        let got = got
            .iter()
            .map(|(k, v)| (k.as_ref(), v.id.as_str()))
            .collect::<HashMap<&str, &str>>();
        assert_eq!(got, want);
    }

    #[test]
    fn test_collect_anchors() {
        let url = Url::parse("http://a.com/schema.json").unwrap();
        let json: Value = serde_json::from_str(
            r#"{
                "$defs": {
                    "s2": {
                        "$id": "http://b.com",
                        "$anchor": "b1", 
                        "items": [
                            { "$anchor": "b2" },
                            {
                                "$id": "http//c.com",
                                "items": [
                                    {"$anchor": "c1"},
                                    {"$dynamicAnchor": "c2"}
                                ]
                            },
                            { "$dynamicAnchor": "b3" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();
        let mut resources = HashMap::new();
        DRAFT2020
            .collect_resources(&json, &url, String::new(), &url, &mut resources)
            .unwrap();
        assert!(resources.get("").unwrap().anchors.is_empty());
        assert_eq!(resources.get("/$defs/s2").unwrap().anchors, {
            let mut want = HashMap::new();
            want.insert("b1".into(), "/$defs/s2".into());
            want.insert("b2".into(), "/$defs/s2/items/0".into());
            want.insert("b3".into(), "/$defs/s2/items/2".into());
            want
        });
        assert_eq!(resources.get("/$defs/s2/items/1").unwrap().anchors, {
            let mut want = HashMap::new();
            want.insert("c1".into(), "/$defs/s2/items/1/items/0".into());
            want.insert("c2".into(), "/$defs/s2/items/1/items/1".into());
            want
        });
    }

    #[test]
    fn test_is_subschema() {
        let tests = vec![("/allOf/0", true)];
        for test in tests {
            let got = DRAFT2020.is_subschema(test.0);
            assert_eq!(got, test.1, "{}", test.0);
        }
    }
}
