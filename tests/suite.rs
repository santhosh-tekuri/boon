use std::{env, error::Error, ffi::OsStr, fs::File, path::Path};

use boon::{Compiler, Draft, Schemas, UrlLoader};
use serde::{Deserialize, Serialize};
use serde_json::Value;

static SKIP: [&str; 2] = [
    "zeroTerminatedFloats.json", // only draft4: this behavior is changed in later drafts
    "float-overflow.json",
];

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
fn test_suites() -> Result<(), Box<dyn Error>> {
    if let Ok(suite) = env::var("TEST_SUITE") {
        test_suite(&suite)?;
    } else {
        test_suite("tests/JSON-Schema-Test-Suite")?;
        test_suite("tests/Extra-Test-Suite")?;
    }
    Ok(())
}

fn test_suite(suite: &str) -> Result<(), Box<dyn Error>> {
    if !Path::new(suite).exists() {
        Err(format!("test suite {suite} does not exist"))?;
    }
    test_dir(suite, "draft4", Draft::V4)?;
    test_dir(suite, "draft6", Draft::V6)?;
    test_dir(suite, "draft7", Draft::V7)?;
    test_dir(suite, "draft2019-09", Draft::V2019_09)?;
    test_dir(suite, "draft2020-12", Draft::V2020_12)?;
    Ok(())
}

fn test_dir(suite: &str, path: &str, draft: Draft) -> Result<(), Box<dyn Error>> {
    let prefix = Path::new(suite).join("tests");
    let dir = prefix.join(path);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in dir.read_dir()? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let tmp_entry_path = entry.path();
        let entry_path = tmp_entry_path.strip_prefix(&prefix)?.to_str().unwrap();
        if file_type.is_file() {
            if !SKIP.iter().any(|n| OsStr::new(n) == entry.file_name()) {
                test_file(suite, entry_path, draft)?;
            }
        } else if file_type.is_dir() {
            test_dir(suite, entry_path, draft)?;
        }
    }
    Ok(())
}

fn test_file(suite: &str, path: &str, draft: Draft) -> Result<(), Box<dyn Error>> {
    println!("FILE: {path}");
    let path = Path::new(suite).join("tests").join(path);
    let optional = path.components().any(|comp| comp.as_os_str() == "optional");
    let file = File::open(path)?;

    let url = "http://testsuite.com/schema.json";
    let groups: Vec<Group> = serde_json::from_reader(file)?;
    for group in groups {
        println!("{}", group.description);
        let mut schemas = Schemas::default();
        let mut compiler = Compiler::default();
        compiler.set_default_draft(draft);
        if optional {
            compiler.enable_format_assertions();
            compiler.enable_content_assertions();
        }
        compiler.register_url_loader("http", Box::new(RemotesLoader(suite.to_owned())));
        compiler.register_url_loader("https", Box::new(RemotesLoader(suite.to_owned())));
        compiler.add_resource(url, group.schema)?;
        let sch_index = compiler.compile(url, &mut schemas)?;
        for test in group.tests {
            println!("    {}", test.description);
            let result = schemas.validate(&test.data, sch_index);
            if let Err(e) = &result {
                for line in format!("{e}").lines() {
                    println!("        {line}");
                }
                for line in format!("{e:#}").lines() {
                    println!("        {line}");
                }
            }
            assert_eq!(result.is_ok(), test.valid);
        }
    }
    Ok(())
}

struct RemotesLoader(String);
impl UrlLoader for RemotesLoader {
    fn load(&self, url: &str) -> Result<Value, Box<dyn std::error::Error>> {
        // remotes folder --
        if let Some(path) = url.strip_prefix("http://localhost:1234/") {
            let path = Path::new(&self.0).join("remotes").join(path);
            let file = File::open(path)?;
            let json: Value = serde_json::from_reader(file)?;
            return Ok(json);
        }
        Err("no internet")?
    }
}
