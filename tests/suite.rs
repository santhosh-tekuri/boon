use std::{fs::File, path::Path};

use jsonschema::{Compiler, Schemas};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    run_file("draft4/type.json");
    run_file("draft4/enum.json");
}

fn run_file(path: &str) {
    let suite = Path::new("tests/JSON-Schema-Test-Suite/tests/");
    let file = File::open(suite.join(path)).unwrap();

    let url = "http://testsuite.com/schema.json";
    let groups: Vec<Group> = serde_json::from_reader(file).unwrap();
    for group in groups {
        println!("{}", group.description);
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        compiler.add_resource(url, group.schema).unwrap();
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
