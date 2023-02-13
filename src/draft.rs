use std::{borrow::Cow, collections::HashMap, str::Utf8Error};

use once_cell::sync::Lazy;
use serde_json::Value;
use url::Url;

use crate::{root::Resource, util::*};

const POS_SELF: u8 = 1 << 0;
const POS_PROP: u8 = 1 << 1;
const POS_ITEM: u8 = 1 << 2;

pub(crate) static DRAFT4: Lazy<Draft> = Lazy::new(|| Draft {
    version: 4,
    id: "id",
    bool_schema: false,
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
    vocab: vec![],
});

pub(crate) static DRAFT6: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT4.subschemas.clone();
    subschemas.extend([("propertyNames", POS_SELF), ("contains", POS_SELF)]);
    Draft {
        version: 6,
        id: "$id",
        bool_schema: true,
        subschemas,
        vocab: vec![],
    }
});

pub(crate) static DRAFT7: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT6.subschemas.clone();
    subschemas.extend([("if", POS_SELF), ("then", POS_SELF), ("else", POS_SELF)]);
    Draft {
        version: 7,
        id: "$id",
        bool_schema: true,
        subschemas,
        vocab: vec![],
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
        bool_schema: true,
        subschemas,
        vocab: vec![
            "https://json-schema.org/draft/2019-09/vocab/core",
            "https://json-schema.org/draft/2019-09/vocab/applicator",
            "https://json-schema.org/draft/2019-09/vocab/validation",
            "https://json-schema.org/draft/2019-09/vocab/meta-data",
            "https://json-schema.org/draft/2019-09/vocab/format",
            "https://json-schema.org/draft/2019-09/vocab/content",
        ],
    }
});

pub(crate) static DRAFT2020: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT2019.subschemas.clone();
    subschemas.extend([("prefixItems", POS_ITEM)]);
    Draft {
        version: 2020,
        id: "$id",
        bool_schema: true,
        subschemas,
        vocab: vec![
            "https://json-schema.org/draft/2020-12/vocab/core",
            "https://json-schema.org/draft/2020-12/vocab/applicator",
            "https://json-schema.org/draft/2020-12/vocab/unevaluated",
            "https://json-schema.org/draft/2020-12/vocab/validation",
            "https://json-schema.org/draft/2020-12/vocab/meta-data",
            "https://json-schema.org/draft/2020-12/vocab/format-annotation",
            "https://json-schema.org/draft/2020-12/vocab/format-assertion",
            "https://json-schema.org/draft/2020-12/vocab/content",
        ],
    }
});

pub(crate) fn latest() -> &'static Draft {
    &DRAFT2020
}

// --

pub(crate) struct Draft {
    pub(crate) version: usize,
    id: &'static str,
    bool_schema: bool,
    subschemas: HashMap<&'static str, u8>,
    vocab: Vec<&'static str>,
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

    fn has_anchor(&self, json: &Value, anchor: &str) -> Result<bool, Utf8Error> {
        let Value::Object(obj) = json else {
            return Ok(false);
        };

        if self.version < 2019 {
            // anchor is specified in id
            if let Some(Value::String(id)) = obj.get(self.id) {
                let (_, fragment) = split(id);
                let Some(got) = fragment_to_anchor(fragment)? else {
                    return Ok(false);
                };
                return Ok(got.as_ref() == anchor);
            }
        }
        if self.version >= 2019 {
            if let Some(Value::String(s)) = obj.get("$anchor") {
                if s == anchor {
                    return Ok(true);
                }
            }
        }
        if self.version >= 2019 {
            if let Some(Value::String(s)) = obj.get("$dynamicAnchor") {
                if s == anchor {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn collect_anchors(
        &self,
        json: &Value,
        ptr: &str,
        res: &mut Resource,
    ) -> Result<(), Utf8Error> {
        let Value::Object(obj) = json else {
            return Ok(());
        };

        if self.version < 2019 {
            // anchor is specified in id
            if let Some(Value::String(id)) = obj.get(self.id) {
                let (_, fragment) = split(id);
                if let Some(anchor) = fragment_to_anchor(fragment)? {
                    res.anchors.insert(anchor.into_owned(), ptr.to_owned());
                };
                return Ok(());
            }
        }
        if self.version >= 2019 {
            if let Some(Value::String(anchor)) = obj.get("$anchor") {
                res.anchors.insert(anchor.to_owned(), ptr.to_owned());
            }
        }
        if self.version >= 2020 {
            if let Some(Value::String(anchor)) = obj.get("$dynamicAnchor") {
                res.anchors.insert(anchor.to_owned(), ptr.to_owned());
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
        resources: &mut HashMap<String, Resource>,
    ) -> Result<(), String> {
        let Value::Object(obj) = json else {
            return Ok(());
        };

        let mut base = Cow::Borrowed(base);
        let id = if self.version < 2019 && obj.contains_key("$ref") {
            // All other properties in a "$ref" object MUST be ignored
            None
        } else {
            obj.get(self.id)
        };
        if let Some(Value::String(obj_id)) = id {
            let (obj_id, _) = split(obj_id);
            let Ok(obj_id) = base.join(obj_id) else {
                return Err(ptr);
            };
            resources.insert(ptr.clone(), Resource::new(obj_id.clone()));
            base = Cow::Owned(obj_id);
        } else if ptr.is_empty() {
            // root resource
            resources.insert(ptr.clone(), Resource::new(base.as_ref().clone()));
        }

        // collect anchors
        if let Some(res) = resources.values_mut().find(|res| res.id == *base.as_ref()) {
            if self.collect_anchors(json, &ptr, res).is_err() {
                return Err(ptr);
            };
        } else {
            debug_assert!(false, "base resource must exist");
        }

        for (&kw, &pos) in &self.subschemas {
            let Some(v) = obj.get(kw) else {
                continue;
            };
            if pos & POS_SELF != 0 {
                let ptr = format!("{ptr}/{kw}");
                self.collect_resources(v, base.as_ref(), ptr, resources)?;
            }
            if pos & POS_ITEM != 0 {
                if let Value::Array(arr) = v {
                    for (i, item) in arr.iter().enumerate() {
                        let ptr = format!("{ptr}/{kw}/{i}");
                        self.collect_resources(item, base.as_ref(), ptr, resources)?;
                    }
                }
            }
            if pos & POS_PROP != 0 {
                if let Value::Object(obj) = v {
                    for (pname, pvalue) in obj {
                        let ptr = format!("{ptr}/{kw}/{}", escape(pname));
                        self.collect_resources(pvalue, base.as_ref(), ptr, resources)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Compiler, Schemas};

    use super::*;

    #[test]
    fn test_meta() {
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        let v: Value = serde_json::from_str(include_str!("metaschemas/draft4.json")).unwrap();
        let url = "https://json-schema.org/draft-04/schema";
        compiler.add_resource(url, v).unwrap();
        compiler.compile(&mut schemas, url.to_owned()).unwrap();
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
        let base = Url::parse("http://a.com/schema.json").unwrap();
        let json: Value = serde_json::from_str(
            &r#"{
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
            .collect_resources(&json, &base, String::new(), &mut got)
            .unwrap();
        let got = got
            .iter()
            .map(|(k, v)| (k.as_ref(), v.id.as_str()))
            .collect::<HashMap<&str, &str>>();
        assert_eq!(got, want);
    }

    #[test]
    fn test_collect_anchors() {
        let base = Url::parse("http://a.com/schema.json").unwrap();
        let json: Value = serde_json::from_str(
            &r#"{
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
            .collect_resources(&json, &base, String::new(), &mut resources)
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
