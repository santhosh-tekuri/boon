use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use crate::{compiler::CompileError, draft::*, util::*};

use serde_json::Value;
use url::Url;

//#[derive(Debug)]
pub(crate) struct Resource {
    pub(crate) draft: &'static Draft,
    pub(crate) ids: HashMap<String, Url>,
    pub(crate) url: Url,
    pub(crate) doc: Value,
}

impl Resource {
    pub(crate) fn check_duplicate_id(&self) -> Result<(), CompileError> {
        let mut set = HashSet::new();
        for url in self.ids.values() {
            if !set.insert(url) {
                return Err(CompileError::DuplicateId {
                    res: self.url.clone(),
                    id: url.clone(),
                });
            }
        }
        Ok(())
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
