use std::{collections::HashMap, error::Error, fs::File};

use boon::{CompileError, Compiler, Schemas, UrlLoader};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Test {
    description: String,
    remotes: Option<HashMap<String, Value>>,
    schema: Value,
    errors: Option<Vec<String>>,
}

#[test]
fn test_invalid_schemas() -> Result<(), Box<dyn Error>> {
    let file = File::open("tests/invalid-schemas.json")?;
    let tests: Vec<Test> = serde_json::from_reader(file)?;
    for test in tests {
        println!("{}", test.description);
        match compile(&test) {
            Ok(_) => {
                if test.errors.is_some() {
                    Err("want compilation to fail")?;
                }
            }
            Err(e) => {
                println!("   {e}");
                let error = format!("{e:?}");
                let Some(errors) = &test.errors else {
                    return Err("want compilation to succeed")?;
                };
                for want in errors {
                    if !error.contains(want) {
                        println!("    got {error}");
                        println!("   want {want}");
                        panic!("error mismatch");
                    }
                }
            }
        }
    }
    Ok(())
}

fn compile(test: &Test) -> Result<(), CompileError> {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let url = "http://fake.com/schema.json";
    if let Some(remotes) = &test.remotes {
        compiler.register_url_loader("http", Box::new(Remotes(remotes.clone())));
    }
    compiler.add_resource(url, test.schema.clone())?;
    compiler.compile(url.to_owned(), &mut schemas)?;
    Ok(())
}

struct Remotes(HashMap<String, Value>);

impl UrlLoader for Remotes {
    fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
        if let Some(v) = self.0.get(url) {
            return Ok(v.clone());
        }
        Err("remote not found")?
    }
}
