use std::{env, fs::File};

use boon::{Compiler, Schemas};
use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::Value;

pub fn validate(c: &mut Criterion) {
    let (Ok(schema), Ok(instance)) = (env::var("SCHEMA"), env::var("INSTANCE")) else {
        panic!("SCHEMA, INSTANCE environment variables not set");
    };

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_format_assertions();
    let sch = compiler.compile(&schema, &mut schemas).unwrap();
    let rdr = File::open(&instance).unwrap();
    let inst: Value = if instance.ends_with(".yaml") || instance.ends_with(".yml") {
        serde_yaml::from_reader(rdr).unwrap()
    } else {
        serde_json::from_reader(rdr).unwrap()
    };
    c.bench_function("boon", |b| b.iter(|| schemas.validate(&inst, sch).unwrap()));
}

criterion_group!(benches, validate);
criterion_main!(benches);
