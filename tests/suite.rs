use std::{fs::File, path::Path};

use jsonschema::{Compiler, Draft, Schemas, UrlLoader};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Host;

const SUITE_DIR: &str = "tests/JSON-Schema-Test-Suite";

#[derive(Debug, Serialize, Deserialize)]
struct Group {
    description: String,
    schema: Value,
    tests: Vec<Test>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Test {
    description: String,
    data: Value,
    valid: bool,
}

#[test]
fn test_suite() {
    run_file("draft4/type.json", Draft::V4);
    run_file("draft4/enum.json", Draft::V4);
    run_file("draft4/minProperties.json", Draft::V4);
    run_file("draft4/maxProperties.json", Draft::V4);
    run_file("draft4/required.json", Draft::V4);
    run_file("draft4/properties.json", Draft::V4);
    run_file("draft4/minItems.json", Draft::V4);
    run_file("draft4/maxItems.json", Draft::V4);
    run_file("draft4/uniqueItems.json", Draft::V4);
    run_file("draft4/minLength.json", Draft::V4);
    run_file("draft4/maxLength.json", Draft::V4);
    run_file("draft4/additionalProperties.json", Draft::V4);
    run_file("draft4/additionalItems.json", Draft::V4);
    run_file("draft4/not.json", Draft::V4);
    run_file("draft4/allOf.json", Draft::V4);
    run_file("draft4/anyOf.json", Draft::V4);
    run_file("draft4/oneOf.json", Draft::V4);
    run_file("draft4/dependencies.json", Draft::V4);
    run_file("draft4/default.json", Draft::V4);
    run_file("draft4/ref.json", Draft::V4);
}

fn run_file(path: &str, draft: Draft) {
    let path = Path::new(SUITE_DIR).join("tests").join(path);
    let file = File::open(path).unwrap();

    let url = "http://testsuite.com/schema.json";
    let groups: Vec<Group> = serde_json::from_reader(file).unwrap();
    for group in groups {
        println!("{}", group.description);
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        compiler.set_default_draft(draft);
        compiler.add_resource(url, group.schema).unwrap();
        compiler.register_url_loader("http", Box::new(HttpUrlLoader));
        let sch_index = compiler.compile(&mut schemas, url.into()).unwrap();
        for test in group.tests {
            println!("    {}", test.description);
            let result = schemas.validate(&test.data, sch_index);
            if let Err(e) = &result {
                println!("        {e:#}");
            }
            assert_eq!(result.is_ok(), test.valid);
        }
    }
}

struct HttpUrlLoader;
impl UrlLoader for HttpUrlLoader {
    fn load(&self, url: &url::Url) -> Result<Value, Box<dyn std::error::Error>> {
        // ensure that url has "localhost:1234"
        if !matches!(url.host(), Some(Host::Domain("localhost"))) {
            Err("no internet")?;
        }
        if !matches!(url.port(), Some(1234)) {
            Err("no internet")?;
        }

        let path = Path::new(SUITE_DIR).join("remotes").join(&url.path()[1..]);
        let file = File::open(path)?;
        let json: Value = serde_json::from_reader(file)?;
        Ok(json)
    }
}
