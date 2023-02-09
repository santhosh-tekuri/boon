use std::{
    collections::{hash_map::Entry, HashMap},
    error::Error,
    fs::File,
    str::FromStr,
};

use crate::util::*;

use serde_json::Value;
use url::Url;

trait ResourceLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError>;
}

// --

#[derive(Debug)]
struct Resource {
    doc: Value,
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

// --

pub struct Resources {
    map: HashMap<Url, Resource>,
    loader: Box<dyn ResourceLoader>,
}

impl Resources {
    fn new() -> Self {
        Self {
            map: Default::default(),
            loader: Box::new(DefaultResourceLoader::new()),
        }
    }

    fn with_loader(loader: Box<dyn ResourceLoader>) -> Self {
        Self {
            map: Default::default(),
            loader,
        }
    }
}

impl Resources {
    fn load_if_absent(&mut self, url: Url) -> Result<&Resource, LoadResourceError> {
        match self.map.entry(url) {
            Entry::Occupied(e) => Ok(e.into_mut()),
            Entry::Vacant(e) => {
                let value = self.loader.load(e.key())?;
                Ok(e.insert(Resource { doc: value }))
            }
        }
    }
}

// --

#[derive(Debug)]
enum LoadResourceError {
    Load(Box<dyn Error>),
    Unsupported,
}

impl<E> From<E> for LoadResourceError
where
    E: Into<Box<dyn Error>>,
{
    fn from(value: E) -> Self {
        LoadResourceError::Load(value.into())
    }
}

// --

struct FileLoader;

impl ResourceLoader for FileLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError> {
        let path = url.to_file_path().map_err(|_| "invalid file path")?;
        let file = File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}

// --

struct DefaultResourceLoader(HashMap<&'static str, Box<dyn ResourceLoader>>);

impl DefaultResourceLoader {
    fn new() -> Self {
        let mut v = Self(Default::default());
        v.0.insert("file", Box::new(FileLoader));
        v
    }
}

impl ResourceLoader for DefaultResourceLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError> {
        match self.0.get(url.scheme()) {
            Some(rl) => rl.load(url),
            None => Err(LoadResourceError::Unsupported),
        }
    }
}

// --

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
        let resource = resources.load_if_absent(url);
        println!("{resource:?}");
    }
}
