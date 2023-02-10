use std::{borrow::Cow, collections::HashMap, str::Utf8Error};

use once_cell::sync::Lazy;
use serde_json::{Map, Value};
use url::Url;

use crate::util::*;

const POS_SELF: u8 = 1 << 0;
const POS_PROP: u8 = 1 << 1;
const POS_ITEM: u8 = 1 << 2;

static DRAFT4: Lazy<Draft> = Lazy::new(|| Draft {
    version: 4,
    id: "id",
    bool_schema: false,
    subschemas: HashMap::from([
        // core
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
});

static DRAFT6: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT4.subschemas.clone();
    subschemas.extend([("propertyNames", POS_SELF), ("contains", POS_SELF)]);
    Draft {
        version: 6,
        id: "$id",
        bool_schema: true,
        subschemas,
    }
});

static DRAFT7: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT6.subschemas.clone();
    subschemas.extend([("if", POS_SELF), ("then", POS_SELF), ("else", POS_SELF)]);
    Draft {
        version: 7,
        id: "$id",
        bool_schema: true,
        subschemas,
    }
});

static DRAFT2019: Lazy<Draft> = Lazy::new(|| {
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
    }
});

static DRAFT2020: Lazy<Draft> = Lazy::new(|| {
    let mut subschemas = DRAFT2019.subschemas.clone();
    subschemas.extend([("prefixItems", POS_ITEM)]);
    Draft {
        version: 2020,
        id: "$id",
        bool_schema: true,
        subschemas,
    }
});

pub(crate) fn latest() -> &'static Draft {
    &DRAFT2020
}

// --

pub(crate) struct Draft {
    version: usize,
    id: &'static str,
    bool_schema: bool,
    subschemas: HashMap<&'static str, u8>,
}

impl Draft {
    pub(crate) fn from_url(mut url: &str) -> Option<&'static Draft> {
        let (_, fragment) = split(url);
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

    // error is json-ptr to invalid id
    pub(crate) fn collect_ids(
        &self,
        json: &Value,
        base: &Url,  // base of json
        ptr: String, // ptr of json
        ids: &mut HashMap<String, Url>,
    ) -> Result<(), String> {
        let Value::Object(obj) = json else {
            return Ok(());
        };

        let mut base = Cow::Borrowed(base);
        if let Some(Value::String(obj_id)) = obj.get(self.id) {
            let (obj_id, _) = split(obj_id);
            let Ok(obj_id) = base.join(obj_id) else {
                return Err(ptr);
            };
            ids.insert(ptr.clone(), obj_id.clone());
            base = Cow::Owned(obj_id);
        }

        for (&kw, &pos) in &self.subschemas {
            let Some(v) = obj.get(kw) else {
                continue;
            };
            if pos & POS_SELF != 0 {
                let ptr = format!("{ptr}/{kw}");
                self.collect_ids(v, base.as_ref(), ptr, ids)?;
            }
            if pos & POS_ITEM != 0 {
                if let Value::Array(arr) = v {
                    for (i, item) in arr.iter().enumerate() {
                        let ptr = format!("{ptr}/{kw}/{i}");
                        self.collect_ids(item, base.as_ref(), ptr, ids)?;
                    }
                }
            }
            if pos & POS_PROP != 0 {
                if let Value::Object(obj) = v {
                    for (pname, pvalue) in obj {
                        let ptr = format!("{ptr}/{kw}/{}", escape(pname));
                        self.collect_ids(pvalue, base.as_ref(), ptr, ids)?;
                    }
                }
            }
        }
        Ok(())
    }

    // returns (Value, json_ptr)
    fn lookup_id<'a>(
        &self,
        id: &Url,   // id to look for
        base: &Url, // base of json
        json: &'a Value,
    ) -> Result<Option<(&'a Value, String)>, url::ParseError> {
        let get_id = |v: &Map<String, Value>| {
            let Some(id) = v.get(self.id) else { return None };
            let Value::String(id) = id else { return None };
            Some(base.join(id))
        };

        let Value::Object(obj) = json else {
            return Ok(None);
        };

        let mut base = Cow::Borrowed(base);
        if let Some(obj_id) = get_id(obj) {
            let obj_id = obj_id?;
            if obj_id == *id {
                return Ok(Some((json, String::new())));
            }
            base = Cow::Owned(obj_id);
        }

        for (&kw, &pos) in &self.subschemas {
            let Some(v) = obj.get(kw) else {
                continue;
            };
            if pos & POS_SELF != 0 {
                if let Some((v, mut ptr)) = self.lookup_id(id, base.as_ref(), v)? {
                    ptr.insert_str(0, &format!("/{kw}"));
                    return Ok(Some((v, ptr)));
                }
            }
            if pos & POS_ITEM != 0 {
                if let Value::Array(arr) = v {
                    for (i, item) in arr.iter().enumerate() {
                        if let Some((v, mut ptr)) = self.lookup_id(id, base.as_ref(), item)? {
                            ptr.insert_str(0, &format!("/{kw}/{i}"));
                            return Ok(Some((v, ptr)));
                        }
                    }
                }
            }
            if pos & POS_PROP != 0 {
                if let Value::Object(obj) = v {
                    for (pname, pvalue) in obj {
                        if let Some((v, mut ptr)) = self.lookup_id(id, base.as_ref(), pvalue)? {
                            ptr.insert_str(0, &format!("/{kw}/{}", escape(pname)));
                            return Ok(Some((v, ptr)));
                        }
                    }
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_url() {
        let tests = [
            ("http://json-schema.org/draft/2020-12/schema", Some(2020)), // http url
            ("https://json-schema.org/draft/2020-12/schema", Some(2020)), // https url
            ("https://json-schema.org/schema", Some(latest().version)),  // latest
            ("https://json-schema.org/%64raft/2020-12/schema", Some(2020)), // percent-encoded
        ];
        for (url, version) in tests {
            let got = Draft::from_url(url).map(|d| d.version);
            assert_eq!(got, version, "for {url}");
        }
    }

    #[test]
    fn test_lookup_id() {
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

        struct Test {
            id: &'static str,
            ptr: &'static str,
        }
        let tests = vec![
            Test {
                id: "http://a.com/schemas/schema.json",
                ptr: "",
            },
            Test {
                id: "http://a.com/definitions/s1",
                ptr: "/definitions/s1",
            },
            Test {
                id: "http://a.com/s2",
                ptr: "/definitions/s2",
            },
            Test {
                id: "http://a.com/schemas/s3",
                ptr: "/definitions/s3/definitions/s1",
            },
            Test {
                id: "http://b.com/item",
                ptr: "/definitions/s3/definitions/s1/items",
            },
            Test {
                id: "http://c.com/item",
                ptr: "/definitions/s2/items/0",
            },
            Test {
                id: "http://d.com/item",
                ptr: "/definitions/s2/items/1",
            },
        ];

        for test in tests {
            match DRAFT4
                .lookup_id(&Url::parse(test.id).unwrap(), &base, &json)
                .expect(&format!("lookup {} failed", test.id))
            {
                Some((_, ptr)) => {
                    assert_eq!(ptr, test.ptr);
                }
                None => {
                    panic!("{} not found", test.id);
                }
            }
        }

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
            .collect_ids(&json, &base, String::new(), &mut got)
            .unwrap();
        let got = got
            .iter()
            .map(|(k, v)| (k.as_ref(), v.as_str()))
            .collect::<HashMap<&str, &str>>();
        assert_eq!(got, want);
    }
}
