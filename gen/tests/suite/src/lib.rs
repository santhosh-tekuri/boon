use boongen::compile;

// todo: track files changes automatically
const _: &[u8] = include_bytes!("../schema.json");

#[compile(file = "schema.json", draft = "4")]
struct Schema;

#[cfg(test)]
mod tests {
    use std::{error::Error, fs::File};

    use serde::Deserialize;
    use serde_json::Value;

    use super::Schema;

    #[derive(Deserialize)]
    struct Test {
        description: String,
        data: Value,
        valid: bool,
    }

    #[test]
    fn test() -> Result<(), Box<dyn Error>> {
        let tests: Vec<Test> = serde_json::from_reader(File::open("tests.json")?)?;
        let sch = Schema::new();
        for test in tests {
            println!("{}", test.description);
            assert_eq!(
                sch.is_valid0(&test.data),
                test.valid,
                "{}",
                test.description
            );
        }
        Ok(())
    }
}
