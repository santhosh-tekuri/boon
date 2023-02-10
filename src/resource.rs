use std::str::FromStr;

use crate::{draft::*, util::*};

use serde_json::Value;
use url::Url;

//#[derive(Debug)]
pub(crate) struct Resource {
    pub(crate) draft: &'static Draft,
    pub(crate) url: Url,
    pub(crate) doc: Value,
}

impl Resource {
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
