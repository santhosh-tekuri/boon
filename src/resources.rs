use std::collections::{HashMap, HashSet};

use crate::{
    compiler::CompileError,
    draft::{latest, Draft},
    loader::{DefaultResourceLoader, ResourceLoader},
    resource::Resource,
    util::*,
};

use serde_json::Value;
use url::Url;

// --

pub(crate) struct Resources {
    default_draft: &'static Draft,
    map: HashMap<Url, Resource>,
    loader: Box<dyn ResourceLoader>,
}

impl Resources {
    fn new() -> Self {
        Self {
            default_draft: latest(),
            map: Default::default(),
            loader: Box::new(DefaultResourceLoader::new()),
        }
    }

    fn with_loader(loader: Box<dyn ResourceLoader>) -> Self {
        Self {
            default_draft: latest(),
            map: Default::default(),
            loader,
        }
    }
}

impl Resources {
    fn load_if_absent(&mut self, url: Url) -> Result<&Resource, CompileError> {
        if let Some(_r) = self.map.get(&url) {
            // return Ok(r); does not work
            // this is current borrow checker limitation
            // see: https://github.com/rust-lang/rust/issues/51545
            // see: https://users.rust-lang.org/t/strange-borrow-checker-behavior-when-returning-content-of-option/88982
            return Ok(self.map.get(&url).unwrap());
        }

        let doc = self.loader.load(&url)?;
        self.add_resource(HashSet::new(), url, doc)
    }

    fn add_resource(
        &mut self,
        mut cycle: HashSet<Url>,
        url: Url,
        doc: Value,
    ) -> Result<&Resource, CompileError> {
        let draft = (|| {
            let Value::Object(obj) = &doc else {
                return Ok(self.default_draft);
            };
            let Some(Value::String(sch)) = obj.get("$schema") else {
                return Ok(self.default_draft);
            };
            if let Some(draft) = Draft::from_url(sch) {
                return Ok(draft);
            }
            let (sch, _) = split(sch);
            let Ok(sch) = Url::parse(sch) else {
                return Err(CompileError::InvalidMetaSchema { resource_url: url.clone()});
            };
            if let Some(r) = self.map.get(&sch) {
                return Ok(r.draft);
            }
            if !cycle.insert(sch.clone()) {
                return Err(CompileError::MetaSchemaCycle { resource_url: sch });
            }
            let doc = self.loader.load(&sch)?;
            Ok(self.add_resource(cycle, sch, doc)?.draft)
        })()?;

        let r = Resource {
            draft,
            url: url.clone(),
            doc,
        };
        Ok(self.map.entry(url).or_insert(r))
    }
}
// --

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_resource_find() {
        let path = fs::canonicalize("test.json").unwrap();
        let url = Url::from_file_path(path).unwrap();
        let mut resources = Resources::new();
        let resource = resources.load_if_absent(url).unwrap();
        println!("{:?}", resource.doc);
    }
}
