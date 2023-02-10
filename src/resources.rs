use std::collections::{HashMap, HashSet};

use crate::{
    compiler::CompileError::{self, *},
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

        let doc = match self.loader.load(&url) {
            Ok(doc) => doc,
            Err(e) => return Err(e.into_compile_error(&url)),
        };
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
                return Err(InvalidMetaSchema { res: url.as_str().to_owned()});
            };
            if let Some(r) = self.map.get(&sch) {
                return Ok(r.draft);
            }
            if !cycle.insert(sch.clone()) {
                return Err(MetaSchemaCycle { res: sch.into() });
            }
            let doc = match self.loader.load(&url) {
                Ok(doc) => doc,
                Err(e) => return Err(e.into_compile_error(&url)),
            };
            Ok(self.add_resource(cycle, sch, doc)?.draft)
        })()?;

        let ids = {
            let mut ids = HashMap::default();
            if let Err(ptr) = draft.collect_ids(&doc, &url, String::new(), &mut ids) {
                let mut url = url;
                url.set_fragment(Some(&ptr));
                return Err(InvalidId { loc: url.into() });
            }
            ids
        };

        let r = Resource {
            draft,
            ids,
            url: url.clone(),
            doc,
        };
        r.check_duplicate_id()?;

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
