use std::{error::Error, fs::File};

use boon::{CompileError, Compiler, Schemas};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Test {
    description: String,
    schema: Value,
    errors: Vec<String>,
}

#[test]
fn test_invalid_schemas() -> Result<(), Box<dyn Error>> {
    let file = File::open("tests/invalid-schemas.json")?;
    let tests: Vec<Test> = serde_json::from_reader(file)?;
    for test in tests {
        println!("{}", test.description);
        match compile_schema(test.schema) {
            Ok(_) => {
                if !test.errors.is_empty() {
                    Err("    expected compilation to fail")?;
                }
            }
            Err(e) => {
                println!("   {e}");
                let error = format!("{e:?}");
                for want in &test.errors {
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

fn compile_schema(v: Value) -> Result<(), CompileError> {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let url = "http://fake.com/schema.json";
    compiler.add_resource(url, v)?;
    compiler.compile(url.to_owned(), &mut schemas)?;
    Ok(())
}
