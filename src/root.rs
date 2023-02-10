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
}

impl Root {
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

    fn base_url(&self, mut ptr: &str) -> &Url {
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

    fn lookup_ptr(&self, ptr: &str) -> Result<Option<&Value>, std::str::Utf8Error> {
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
}

#[derive(Debug)]
pub(crate) struct Resource {
    pub(crate) id: Url,
    pub(crate) anchors: HashMap<String, String>, // anchor => ptr
}

impl Resource {
    pub(crate) fn new(id: Url) -> Self {
        Self {
            id,
            anchors: HashMap::new(),
        }
    }
}
