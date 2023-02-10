use std::{collections::HashMap, error::Error, fs::File};

use serde_json::Value;
use url::Url;

use crate::compiler::CompileError;

pub trait UrlLoader {
    fn load(&self, url: &Url) -> Result<Value, UrlLoadError>;
}

// --

#[derive(Debug)]
pub enum UrlLoadError {
    Loading(Box<dyn Error>),
    Unsupported,
}

impl UrlLoadError {
    pub(crate) fn into_compile_error(self, res: &Url) -> CompileError {
        let res = res.as_str().to_owned();
        match self {
            Self::Loading(src) => CompileError::LoadUrlError { res, src },
            Self::Unsupported => CompileError::UnsupportedUrl { res },
        }
    }
}

impl<E> From<E> for UrlLoadError
where
    E: Into<Box<dyn Error>>,
{
    fn from(value: E) -> Self {
        UrlLoadError::Loading(value.into())
    }
}

// --

struct FileLoader;

impl UrlLoader for FileLoader {
    fn load(&self, url: &Url) -> Result<Value, UrlLoadError> {
        let path = url.to_file_path().map_err(|_| "invalid file path")?;
        let file = File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}

// --

pub struct DefaultUrlLoader(HashMap<&'static str, Box<dyn UrlLoader>>);

impl DefaultUrlLoader {
    pub fn new() -> Self {
        let mut v = Self(Default::default());
        v.0.insert("file", Box::new(FileLoader));
        v
    }

    pub fn register(&mut self, schema: &'static str, loader: Box<dyn UrlLoader>) {
        self.0.insert(schema, loader);
    }
}

impl UrlLoader for DefaultUrlLoader {
    fn load(&self, url: &Url) -> Result<Value, UrlLoadError> {
        match self.0.get(url.scheme()) {
            Some(rl) => rl.load(url),
            None => Err(UrlLoadError::Unsupported),
        }
    }
}

// --

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_default_url_loader() {
        let path = fs::canonicalize("test.json").unwrap();
        let url = Url::from_file_path(path).unwrap();
        let doc = DefaultUrlLoader::new().load(&url).unwrap();
        println!("{:?}", doc);
    }
}
