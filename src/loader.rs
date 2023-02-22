use std::{collections::HashMap, error::Error, fs::File};

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
        match self.0.get(url.scheme()) {
            Some(rl) => rl.load(url).map_err(|src| CompileError::LoadUrlError {
                url: url.as_str().to_owned(),
                src,
            }),
            None => Err(CompileError::UnsupportedUrlScheme {
                url: url.as_str().to_owned(),
            }),
        }
    }
}
