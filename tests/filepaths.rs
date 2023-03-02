use std::fs;

use boon::{CompileError, Compiler, Schemas};

fn test(path: &str) -> Result<(), CompileError> {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.compile(path, &mut schemas)?;
    Ok(())
}

#[test]
fn test_absolute() -> Result<(), CompileError> {
    let path = fs::canonicalize("tests/examples/schema.json").unwrap();
    test(path.to_string_lossy().as_ref())
}

#[test]
fn test_relative_slash() -> Result<(), CompileError> {
    test("tests/examples/schema.json")
}

#[test]
#[cfg(windows)]
fn test_relative_backslash() -> Result<(), CompileError> {
    test("tests\\examples\\schema.json")
}

#[test]
fn test_absolutei_space() -> Result<(), CompileError> {
    let path = fs::canonicalize("tests/examples/sample schema.json").unwrap();
    test(path.to_string_lossy().as_ref())
}

#[test]
fn test_relative_slash_space() -> Result<(), CompileError> {
    test("tests/examples/sample schema.json")
}

#[test]
#[cfg(windows)]
fn test_relative_backslash_space() -> Result<(), CompileError> {
    test("tests\\examples\\sample schema.json")
}
