use std::{collections::HashMap, error::Error, fs::File};

use once_cell::sync::Lazy;
use serde_json::Value;
use url::Url;

use crate::compiler::CompileError;

pub trait UrlLoader {
    fn load(&self, url: &Url) -> Result<Value, Box<dyn Error>>;
}

// --

struct FileLoader;

impl UrlLoader for FileLoader {
    fn load(&self, url: &Url) -> Result<Value, Box<dyn Error>> {
        let path = url.to_file_path().map_err(|_| "invalid file path")?;
        let file = File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}

// --

pub(crate) struct DefaultUrlLoader(HashMap<&'static str, Box<dyn UrlLoader>>);

impl DefaultUrlLoader {
    pub fn new() -> Self {
        let mut v = Self(Default::default());
        v.0.insert("file", Box::new(FileLoader));
        v
    }

    pub fn register(&mut self, schema: &'static str, loader: Box<dyn UrlLoader>) {
        self.0.insert(schema, loader);
    }

    pub(crate) fn load(&self, url: &Url) -> Result<Value, CompileError> {
        // check in STD_METAFILES
        let meta = url
            .as_str()
            .strip_prefix("http://json-schema.org/")
            .or_else(|| url.as_str().strip_prefix("https://json-schema.org/"));
        if let Some(meta) = meta {
            if let Some(content) = STD_METAFILES.get(meta) {
                return serde_json::from_str::<Value>(content).map_err(|e| {
                    CompileError::LoadUrlError {
                        url: url.to_string(),
                        src: e.into(),
                    }
                });
            }
        }

        match self.0.get(url.scheme()) {
            Some(loader) => loader.load(url).map_err(|src| CompileError::LoadUrlError {
                url: url.as_str().to_owned(),
                src,
            }),
            None => Err(CompileError::UnsupportedUrlScheme {
                url: url.as_str().to_owned(),
            }),
        }
    }
}

pub(crate) static STD_METAFILES: Lazy<HashMap<String, &str>> = Lazy::new(|| {
    let mut files = HashMap::new();
    macro_rules! add {
        ($path:expr) => {
            files.insert(
                $path["metaschemas/".len()..].to_owned(),
                include_str!($path),
            );
        };
    }
    add!("metaschemas/draft-04/schema");
    add!("metaschemas/draft-06/schema");
    add!("metaschemas/draft-07/schema");
    add!("metaschemas/draft/2019-09/schema");
    add!("metaschemas/draft/2019-09/meta/core");
    add!("metaschemas/draft/2019-09/meta/applicator");
    add!("metaschemas/draft/2019-09/meta/validation");
    add!("metaschemas/draft/2019-09/meta/meta-data");
    add!("metaschemas/draft/2019-09/meta/format");
    add!("metaschemas/draft/2019-09/meta/content");
    add!("metaschemas/draft/2020-12/schema");
    add!("metaschemas/draft/2020-12/meta/core");
    add!("metaschemas/draft/2020-12/meta/applicator");
    add!("metaschemas/draft/2020-12/meta/unevaluated");
    add!("metaschemas/draft/2020-12/meta/validation");
    add!("metaschemas/draft/2020-12/meta/meta-data");
    add!("metaschemas/draft/2020-12/meta/content");
    add!("metaschemas/draft/2020-12/meta/format-annotation");
    add!("metaschemas/draft/2020-12/meta/format-assertion");
    files
});
