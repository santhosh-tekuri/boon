use std::collections::{HashMap, HashSet};

use crate::{
    compiler::CompileError::{self, *},
    draft::{latest, Draft},
    loader::DefaultUrlLoader,
    root::Root,
    util::*,
};

use serde_json::Value;
use url::Url;

// --

pub(crate) struct Roots {
    pub(crate) default_draft: &'static Draft,
    map: HashMap<Url, Root>,
    pub(crate) loader: DefaultUrlLoader,
}

impl Roots {
    fn new() -> Self {
        Self {
            default_draft: latest(),
            map: Default::default(),
            loader: DefaultUrlLoader::new(),
        }
    }
}

impl Default for Roots {
    fn default() -> Self {
        Self::new()
    }
}

impl Roots {
    pub(crate) fn get(&self, url: &Url) -> Option<&Root> {
        self.map.get(url)
    }

    pub(crate) fn or_insert(&mut self, mut url: Url, doc: Value) -> Result<bool, CompileError> {
        url.set_fragment(None);
        if !self.map.contains_key(&url) {
            self.add_root(HashSet::new(), url, doc)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn or_load(&mut self, url: Url) -> Result<&Root, CompileError> {
        debug_assert!(url.fragment().is_none(), "trying to add root with fragment");
        if let Some(_r) = self.map.get(&url) {
            // return Ok(r); does not work
            // this is current borrow checker limitation
            // see: https://github.com/rust-lang/rust/issues/51545
            // see: https://users.rust-lang.org/t/strange-borrow-checker-behavior-when-returning-content-of-option/88982
            return Ok(self.map.get(&url).unwrap());
        }

        let doc = self.loader.load(&url)?;
        self.add_root(HashSet::new(), url, doc)
    }

    fn add_root(
        &mut self,
        mut cycle: HashSet<Url>,
        url: Url,
        doc: Value,
    ) -> Result<&Root, CompileError> {
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
                return Err(InvalidMetaSchema { url: url.as_str().to_owned()});
            };
            if let Some(r) = self.map.get(&sch) {
                return Ok(r.draft);
            }
            if !cycle.insert(sch.clone()) {
                return Err(MetaSchemaCycle { url: sch.into() });
            }
            let doc = self.loader.load(&url)?;
            Ok(self.add_root(cycle, sch, doc)?.draft)
        })()?;

        let ids = {
            let mut ids = HashMap::default();
            if let Err(ptr) = draft.collect_resources(&doc, &url, String::new(), &mut ids) {
                let mut url = url;
                url.set_fragment(Some(&ptr));
                return Err(InvalidId { loc: url.into() });
            }
            ids
        };

        let r = Root {
            draft,
            resources: ids,
            url: url.clone(),
            doc,
        };
        r.check_duplicate_id()?;

        Ok(self.map.entry(url).or_insert(r))
    }
}
