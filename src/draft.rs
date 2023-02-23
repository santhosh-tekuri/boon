use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap},
};

use once_cell::sync::Lazy;
use serde_json::Value;
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
        if !fragment.is_empty() {
            return None;
        }
        if let Some(s) = url.strip_prefix("http://") {
            url = s;
        }
        if let Some(s) = url.strip_prefix("https://") {
            url = s;
        }
        let Ok(url) = path_unescape(url) else {
            return None;
        };
        match url.as_str() {
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

    fn collect_anchors(
        &self,
        json: &Value,
        base: &Url,
        ptr: &str,
        res: &mut Resource,
        root_url: &Url,
    ) -> Result<(), CompileError> {
        let Value::Object(obj) = json else {
            return Ok(());
        };

        let mut add_anchor = |anchor: String| match res.anchors.entry(anchor) {
            Entry::Occupied(entry) => {
                return Err(CompileError::DuplicateAnchor {
                    url: root_url.as_str().to_owned(),
                    anchor: entry.key().to_owned(),
                    ptr1: entry.get().to_owned(),
                    ptr2: ptr.to_owned(),
                });
            }
            entry => {
                entry.or_insert(ptr.to_owned());
                Ok(())
            }
        };

        if self.version < 2019 {
            if obj.contains_key("$ref") {
                return Ok(()); // All other properties in a "$ref" object MUST be ignored
            }
            // anchor is specified in id
            if let Some(Value::String(id)) = obj.get(self.id) {
                let (_, fragment) = split(id);
                let Ok(anchor) = fragment_to_anchor(fragment) else {
                    let mut url = base.clone();
                    url.set_fragment(Some(ptr));
                    return Err(CompileError::InvalidAnchor { loc: url.into() });
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
        json: &Value,
        base: &Url,  // base of json
        ptr: String, // ptr of json
        root_url: &Url,
        resources: &mut HashMap<String, Resource>,
    ) -> Result<(), CompileError> {
        let Value::Object(obj) = json else {
            return Ok(());
        };
        //todo: shouldn't we add resource for root boolean schema?? think

        let id = if self.version < 2019 {
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
        };

        let mut base = Cow::Borrowed(base);
        if let Some(Value::String(obj_id)) = id {
            let (obj_id, _) = split(obj_id);
            let Ok(obj_id) = base.join(obj_id) else {
                let mut url = base.into_owned();
                url.set_fragment(Some(&ptr));
                return Err(CompileError::InvalidId { loc: url.into() });
            };
            resources.insert(ptr.clone(), Resource::new(obj_id.clone()));
            base = Cow::Owned(obj_id);
        } else if ptr.is_empty() {
            // root resource
            resources.insert(ptr.clone(), Resource::new(base.as_ref().clone()));
        }

        // collect anchors
        if let Some(res) = resources.values_mut().find(|res| res.id == *base.as_ref()) {
            self.collect_anchors(json, &base, &ptr, res, root_url)?;
        } else {
            debug_assert!(false, "base resource must exist");
        }

        for (&kw, &pos) in &self.subschemas {
            let Some(v) = obj.get(kw) else {
                continue;
            };
            if pos & POS_SELF != 0 {
                let ptr = format!("{ptr}/{kw}");
                self.collect_resources(v, base.as_ref(), ptr, root_url, resources)?;
            }
            if pos & POS_ITEM != 0 {
                if let Value::Array(arr) = v {
                    for (i, item) in arr.iter().enumerate() {
                        let ptr = format!("{ptr}/{kw}/{i}");
                        self.collect_resources(item, base.as_ref(), ptr, root_url, resources)?;
                    }
                }
            }
            if pos & POS_PROP != 0 {
                if let Value::Object(obj) = v {
                    for (pname, pvalue) in obj {
                        let ptr = format!("{ptr}/{kw}/{}", escape(pname));
                        self.collect_resources(pvalue, base.as_ref(), ptr, root_url, resources)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn load_std_metaschemas() -> Result<Schemas, CompileError> {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_format_assertions();
    compiler.compile(
        "https://json-schema.org/draft/2020-12/schema#".to_string(),
        &mut schemas,
    )?;
    compiler.compile(
        "https://json-schema.org/draft/2019-09/schema#".to_string(),
        &mut schemas,
    )?;
    compiler.compile(
        "http://json-schema.org/draft-07/schema#".to_string(),
        &mut schemas,
    )?;
    compiler.compile(
        "http://json-schema.org/draft-06/schema#".to_string(),
        &mut schemas,
    )?;
    compiler.compile(
        "http://json-schema.org/draft-04/schema#".to_string(),
        &mut schemas,
    )?;
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
        compiler.compile(url.to_owned(), &mut schemas).unwrap();
    }

    #[test]
    fn test_from_url() {
        let tests = [
            ("http://json-schema.org/draft/2020-12/schema", Some(2020)), // http url
            ("https://json-schema.org/draft/2020-12/schema", Some(2020)), // https url
            ("https://json-schema.org/schema", Some(latest().version)),  // latest
            ("https://json-schema.org/draft-04/schema", Some(4)),
            ("https://json-schema.org/%64raft/2020-12/schema", Some(2020)), // percent-encoded
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
}
