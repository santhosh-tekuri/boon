use std::collections::{HashMap, HashSet};

use crate::{compiler::CompileError, draft::*, loader::DefaultUrlLoader, root::Root, util::*};

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

    pub(crate) fn resolve_fragment(&mut self, uf: UrlFrag) -> Result<UrlPtr, CompileError> {
        self.or_load(uf.url.clone())?;
        let Some(root) = self.map.get(&uf.url) else {
            return Err(CompileError::Bug("or_load didn't add".into()));
        };
        root.resolve_fragment(&uf.frag)
    }

    pub(crate) fn ensure_subschema(&mut self, up: &UrlPtr) -> Result<(), CompileError> {
        self.or_load(up.url.clone())?;
        let Some(root) = self.map.get_mut(&up.url) else {
            return Err(CompileError::Bug("or_load didn't add".into()));
        };
        if !root.draft.is_subschema(up.ptr.as_str()) {
            let doc = self.loader.load(&root.url)?;
            let v = up.ptr.lookup(doc, &up.url)?;
            root.draft.validate(up, v)?;
            root.add_subschema(doc, &up.ptr)?;
        }
        Ok(())
    }

    pub(crate) fn or_load(&mut self, url: Url) -> Result<(), CompileError> {
        debug_assert!(url.fragment().is_none(), "trying to add root with fragment");
        if !self.map.contains_key(&url) {
            let doc = self.loader.load(&url)?;
            Roots::add_root(self.default_draft, &mut self.map, &self.loader, url, doc)?;
        }
        Ok(())
    }

    fn add_root<'a>(
        default_draft: &'static Draft,
        wmap: &'a mut HashMap<Url, Root>,
        loader: &DefaultUrlLoader,
        url: Url,
        doc: &Value,
    ) -> Result<&'a Root, CompileError> {
        let draft = {
            let up = UrlPtr {
                url: url.clone(),
                ptr: "".into(),
            };
            loader.get_draft(&up, doc, default_draft, HashSet::new())?
        };
        let vocabs = loader.get_meta_vocabs(doc, draft)?;
        let resources = {
            let mut m = HashMap::default();
            draft.collect_resources(doc, &url, "".into(), &url, &mut m)?;
            m
        };

        if !matches!(url.host_str(), Some("json-schema.org")) {
            draft.validate(
                &UrlPtr {
                    url: url.clone(),
                    ptr: "".into(),
                },
                doc,
            )?;
        }

        let r = Root {
            draft,
            resources,
            url: url.clone(),
            meta_vocabs: vocabs,
        };
        Ok(wmap.entry(url).or_insert(r))
    }

    pub(crate) fn enqueue_root<'a>(
        &self,
        url: Url,
        target: &'a mut HashMap<Url, Root>,
    ) -> Result<&'a Root, CompileError> {
        let doc = self.loader.load(&url)?;
        Self::add_root(self.default_draft, target, &self.loader, url, doc)
    }

    pub(crate) fn insert(&mut self, roots: &mut HashMap<Url, Root>) {
        self.map.extend(roots.drain());
    }
}
