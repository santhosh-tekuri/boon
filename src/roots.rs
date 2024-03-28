use std::collections::{HashMap, HashSet};

use crate::{
    compiler::CompileError::{self, *},
    draft::*,
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
            root.add_subschema(&up.ptr)?;
        }
        Ok(())
    }

    pub(crate) fn or_load(&mut self, url: Url) -> Result<(), CompileError> {
        debug_assert!(url.fragment().is_none(), "trying to add root with fragment");
        if !self.map.contains_key(&url) {
            let doc = self.loader.load(&url)?;
            Roots::add_root(
                self.default_draft,
                &HashMap::new(),
                &mut self.map,
                &self.loader,
                HashSet::new(),
                url,
                doc,
            )?;
        }
        Ok(())
    }

    fn add_root<'a>(
        default_draft: &'static Draft,
        rmap: &HashMap<Url, Root>,
        wmap: &'a mut HashMap<Url, Root>,
        loader: &DefaultUrlLoader,
        mut cycle: HashSet<Url>,
        url: Url,
        doc: Value,
    ) -> Result<&'a Root, CompileError> {
        let (draft, vocabs) = (|| {
            let Value::Object(obj) = &doc else {
                return Ok((default_draft, None));
            };
            let Some(Value::String(sch)) = obj.get("$schema") else {
                return Ok((default_draft, None));
            };
            if let Some(draft) = Draft::from_url(sch) {
                return Ok((draft, None));
            }
            let (sch, _) = split(sch);
            let sch = Url::parse(sch).map_err(|e| InvalidMetaSchemaUrl {
                url: url.as_str().to_owned(),
                src: e.into(),
            })?;
            if let Some(r) = rmap.get(&sch) {
                return Ok((r.draft, r.get_reqd_vocabs()?));
            }
            if let Some(r) = wmap.get(&sch) {
                return Ok((r.draft, r.get_reqd_vocabs()?));
            }
            if sch == url {
                return Err(UnsupportedDraft { url: sch.into() });
            }
            if !cycle.insert(sch.clone()) {
                return Err(MetaSchemaCycle { url: sch.into() });
            }
            let doc = loader.load(&sch)?;
            let meta_root = Roots::add_root(default_draft, rmap, wmap, loader, cycle, sch, doc)?;
            Ok((meta_root.draft, meta_root.get_reqd_vocabs()?))
        })()?;

        let resources = {
            let mut m = HashMap::default();
            draft.collect_resources(&doc, &url, "".into(), &url, &mut m)?;
            m
        };

        if !matches!(url.host_str(), Some("json-schema.org")) {
            if let Some(std_sch) = draft.get_schema() {
                STD_METASCHEMAS.validate(&doc, std_sch).map_err(|src| {
                    CompileError::ValidationError {
                        url: url.to_string(),
                        src: src.clone_static(),
                    }
                })?;
            } else {
                return Err(CompileError::Bug(
                    format!("no metaschema preloaded for draft {}", draft.version).into(),
                ));
            }
        }

        let r = Root {
            draft,
            resources,
            url: url.clone(),
            doc,
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
        Self::add_root(
            self.default_draft,
            &self.map,
            target,
            &self.loader,
            HashSet::new(),
            url,
            doc,
        )
    }

    pub(crate) fn insert(&mut self, roots: &mut HashMap<Url, Root>) {
        self.map.extend(roots.drain());
    }
}
