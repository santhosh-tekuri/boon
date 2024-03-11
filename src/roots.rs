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

    pub(crate) fn or_insert(&mut self, mut url: Url, doc: Value) -> Result<bool, CompileError> {
        url.set_fragment(None);
        if !self.map.contains_key(&url) {
            self.add_root(HashSet::new(), url, doc)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn or_load(&mut self, url: Url) -> Result<(), CompileError> {
        debug_assert!(url.fragment().is_none(), "trying to add root with fragment");
        if !self.map.contains_key(&url) {
            let doc = self.loader.load(&url)?;
            self.add_root(HashSet::new(), url, doc)?;
        }
        Ok(())
    }

    fn add_root(
        &mut self,
        mut cycle: HashSet<Url>,
        url: Url,
        doc: Value,
    ) -> Result<&Root, CompileError> {
        let (draft, vocabs) = (|| {
            let Value::Object(obj) = &doc else {
                return Ok((self.default_draft, None));
            };
            let Some(Value::String(sch)) = obj.get("$schema") else {
                return Ok((self.default_draft, None));
            };
            if let Some(draft) = Draft::from_url(sch) {
                return Ok((draft, None));
            }
            let (sch, _) = split(sch);
            let sch = Url::parse(sch).map_err(|e| InvalidMetaSchemaUrl {
                url: url.as_str().to_owned(),
                src: e.into(),
            })?;
            if let Some(r) = self.map.get(&sch) {
                return Ok((r.draft, r.get_reqd_vocabs()?));
            }
            if !cycle.insert(sch.clone()) {
                if sch == url {
                    return Err(UnsupportedDraft { url: sch.into() });
                } else {
                    return Err(MetaSchemaCycle { url: sch.into() });
                }
            }
            let doc = self.loader.load(&sch)?;
            let meta_root = &self.add_root(cycle, sch, doc)?;
            Ok((meta_root.draft, meta_root.get_reqd_vocabs()?))
        })()?;

        let resources = {
            let mut m = HashMap::default();
            draft.collect_resources(&doc, &url, String::new(), &url, &mut m)?;
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
        r.check_duplicate_id()?;

        Ok(self.map.entry(url).or_insert(r))
    }
}
