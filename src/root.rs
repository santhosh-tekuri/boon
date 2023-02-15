use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use crate::{compiler::CompileError, draft::*, util::*};

use serde_json::Value;
use url::Url;

//#[derive(Debug)]
pub(crate) struct Root {
    pub(crate) draft: &'static Draft,
    pub(crate) resources: HashMap<String, Resource>, // ptr => _
    pub(crate) url: Url,
    pub(crate) doc: Value,
    pub(crate) meta_vocabs: Option<Vec<String>>,
}

impl Root {
    pub(crate) fn has_vocab(&self, name: &str) -> bool {
        if self.draft.version < 2019 || name == "core" {
            return true;
        }
        if let Some(vocabs) = &self.meta_vocabs {
            return vocabs.iter().any(|s| s == name);
        }
        self.draft.default_vocabs.contains(&name)
    }

    pub(crate) fn check_duplicate_id(&self) -> Result<(), CompileError> {
        let mut set = HashSet::new();
        for Resource { id, .. } in self.resources.values() {
            if !set.insert(id) {
                return Err(CompileError::DuplicateId {
                    url: self.url.as_str().to_owned(),
                    id: id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    // resolves `loc` to root-url#json-pointer
    pub(crate) fn resolve(&self, loc: &str) -> Result<String, CompileError> {
        let (url, ptr) = split(loc);

        // look for resource with id==url
        let entry = self
            .resources
            .iter()
            .find(|(_res_ptr, res)| res.id.as_str() == url);
        let Some((res_ptr, res)) = entry else {
            return Ok(loc.to_owned()); // external url
        };

        let anchor = fragment_to_anchor(ptr).map_err(|e| CompileError::ParseUrlError {
            url: loc.to_owned(),
            src: e.into(),
        })?;

        if let Some(anchor) = anchor {
            if let Some(anchor_ptr) = res.anchors.get(anchor.as_ref()) {
                return Ok(format!("{}#{}", self.url, anchor_ptr));
            } else {
                return Err(CompileError::AnchorNotFound {
                    schema_url: self.url.as_str().to_owned(),
                    anchor_url: loc.to_owned(),
                });
            }
        } else {
            return Ok(format!("{}#{}{}", self.url, res_ptr, ptr));
        }
    }

    pub(crate) fn resource(&self, mut ptr: &str) -> Option<&Resource> {
        loop {
            if let Some(res) = self.resources.get(ptr) {
                return Some(res);
            }
            let Some(slash) = ptr.rfind('/') else {
                break;
            };
            ptr = &ptr[..slash];
        }
        None
    }

    pub(crate) fn base_url(&self, mut ptr: &str) -> &Url {
        loop {
            if let Some(Resource { id, .. }) = self.resources.get(ptr) {
                return id;
            }
            let Some(slash) = ptr.rfind('/') else {
                break;
            };
            ptr = &ptr[..slash];
        }
        &self.url
    }

    pub(crate) fn lookup_ptr(&self, ptr: &str) -> Result<Option<&Value>, std::str::Utf8Error> {
        let mut v = &self.doc;
        for tok in ptr_tokens(ptr) {
            let tok = tok?;
            match v {
                Value::Object(obj) => {
                    if let Some(pvalue) = obj.get(&tok) {
                        v = pvalue;
                        continue;
                    }
                }
                Value::Array(arr) => {
                    if let Ok(i) = usize::from_str(&tok) {
                        if let Some(item) = arr.get(i) {
                            v = item;
                            continue;
                        }
                    };
                }
                _ => {}
            }
            return Ok(None);
        }
        Ok(Some(v))
    }

    pub(crate) fn get_vocabs(&self) -> Result<Option<Vec<String>>, CompileError> {
        if self.draft.version < 2019 {
            return Ok(None);
        }
        match &self.doc {
            Value::Object(obj) => {
                if let Some(Value::Object(obj)) = obj.get("$vocabulary") {
                    let mut vocabs = vec![];
                    for vocab in obj.keys() {
                        let name = vocab
                            .strip_prefix(self.draft.vocab_prefix)
                            .filter(|name| self.draft.all_vocabs.contains(name));
                        if let Some(name) = name {
                            vocabs.push(name.to_owned()); // todo: avoid alloc
                        } else {
                            return Err(CompileError::UnsupprtedVocabulary {
                                url: self.url.as_str().to_owned(),
                                vocabulary: vocab.to_owned(),
                            });
                        }
                    }
                    Ok(Some(vocabs))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Resource {
    pub(crate) id: Url,
    pub(crate) anchors: HashMap<String, String>, // anchor => ptr
    pub(crate) dynamic_anchors: HashSet<String>,
}

impl Resource {
    pub(crate) fn new(id: Url) -> Self {
        Self {
            id,
            anchors: HashMap::new(),
            dynamic_anchors: HashSet::new(),
        }
    }
}
