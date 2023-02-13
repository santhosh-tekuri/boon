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

    pub(crate) fn lookup(&self, loc: &str) -> Result<Option<&Value>, CompileError> {
        let (url, ptr) = split(loc);

        // look for subresource with id==url
        let entry = self
            .resources
            .iter()
            .find(|(ptr, res)| res.id.as_str() == url);
        let Some((res_ptr, res)) = entry else {
            return Ok(None);
        };

        let anchor = fragment_to_anchor(ptr).map_err(|e| CompileError::ParseUrlError {
            url: loc.to_owned(),
            src: e.into(),
        })?;
        if let Some(anchor) = anchor {
            let Some(anchor_ptr) = res.anchors.get(anchor.as_ref()) else {
                return Err(CompileError::UrlFragmentNotFound(loc.to_owned()))
            };
            return self
                .lookup_ptr(anchor_ptr)
                .map_err(|e| CompileError::Bug(e.into()));
        }

        let value =
            self.lookup(&format!("{res_ptr}{ptr}"))
                .map_err(|e| CompileError::ParseUrlError {
                    url: loc.to_owned(),
                    src: e.into(),
                })?;
        match value {
            Some(value) => Ok(Some(value)),
            None => Err(CompileError::UrlFragmentNotFound(loc.to_owned())),
        }
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
