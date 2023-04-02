use std::{env, fs::File};

use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::Value;
use suite::Schema;

pub fn validate(c: &mut Criterion) {
    let Ok(instance) = env::var("INSTANCE") else {
         panic!("env INSTANCE is not set"); 
    };
    let rdr = File::open(&instance).unwrap();
    let inst: Value = if instance.ends_with(".yaml") || instance.ends_with(".yml") {
        serde_yaml::from_reader(rdr).unwrap()
    } else {
        serde_json::from_reader(rdr).unwrap()
    };

    let schema = Schema::new();
    c.bench_function("is_valid", |b| b.iter(|| schema.is_valid(&inst)));
}

criterion_group!(benches, validate);
criterion_main!(benches);
