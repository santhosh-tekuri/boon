[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Crates.io](https://img.shields.io/crates/v/boon.svg)](https://crates.io/crates/boon)
[![docs.rs](https://docs.rs/boon/badge.svg)](https://docs.rs/boon/)
[![Build Status](https://github.com/santhosh-tekuri/boon/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/santhosh-tekuri/boon/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/santhosh-tekuri/boon/branch/main/graph/badge.svg?token=A2YC4A0BLG)](https://codecov.io/gh/santhosh-tekuri/boon)
[![dependency status](https://deps.rs/repo/github/Santhosh-tekuri/boon/status.svg?refresh)](https://deps.rs/repo/github/Santhosh-tekuri/boon)

[Examples](https://github.com/santhosh-tekuri/boon/blob/main/tests/examples.rs)
[Changelog](https://github.com/santhosh-tekuri/boon/blob/main/CHANGELOG.md)

## Library Features

- [x] pass [JSON-Schema-Test-Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite) excluding optional(compare with other impls at [bowtie](https://bowtie-json-schema.github.io/bowtie/#))
  - [x] [![draft-04](https://img.shields.io/endpoint?url=https://bowtie.report/badges/rust-boon/compliance/draft4.json)](https://bowtie.report/#/dialects/draft4)
  - [x] [![draft-06](https://img.shields.io/endpoint?url=https://bowtie.report/badges/rust-boon/compliance/draft6.json)](https://bowtie.report/#/dialects/draft6)
  - [x] [![draft-07](https://img.shields.io/endpoint?url=https://bowtie.report/badges/rust-boon/compliance/draft7.json)](https://bowtie.report/#/dialects/draft7)
  - [x] [![draft/2019-09](https://img.shields.io/endpoint?url=https://bowtie.report/badges/rust-boon/compliance/draft2019-09.json)](https://bowtie.report/#/dialects/draft2019-09)
  - [x] [![draft/2020-12](https://img.shields.io/endpoint?url=https://bowtie.report/badges/rust-boon/compliance/draft2020-12.json)](https://bowtie.report/#/dialects/draft2020-12)
- [x] detect infinite loop traps
  - [x] `$schema` cycle
  - [x] validation cycle
- [x] custom `$schema` url
- [x] vocabulary based validation
- [x] ECMA-262 regex compatibility (pass tests from `optional/ecmascript-regex.json`)
- [x] format assertions
  - [x] flag to enable in draft >= 2019-09
  - [x] custom format registration
  - [x] built-in formats
    - [x] regex, uuid
    - [x] ipv4, ipv6
    - [x] hostname, email
    - [x] idn-hostname, idn-email
    - [x] date, time, date-time, duration
    - [x] json-pointer, relative-json-pointer
    - [x] uri, uri-reference, uri-template
    - [x] iri, iri-reference
    - [x] period
- [x] content assertions
  - [x] flag to enable in draft >= 7
  - [x] contentEncoding
    - [x] base64
    - [x] custom
  - [x] contentMediaType
    - [x] application/json
    - [x] custom
  - [x] contentSchema
- [x] errors
  - [x] introspectable
  - [x] hierarchy
    - [x] alternative display with `#`
  - [x] output
    - [x] flag
    - [x] basic
    - [x] detailed
- [ ] custom vocabulary

## CLI

to install: `cargo install --example boon boon`

```
Usage: boon [OPTIONS] SCHEMA [INSTANCE...]

Options:
    -h, --help          Print help information
    -q, --quiet         Do not print errors
    -d, --draft <VER>   Draft used when '$schema' is missing. Valid values 4,
                        6, 7, 2019, 2020 (default 2020)
    -o, --output <FMT>  Output format. Valid values simple, alt, flag, basic,
                        detailed (default simple)
    -f, --assert-format
                        Enable format assertions with draft >= 2019
    -c, --assert-content
                        Enable content assertions with draft >= 7
```

This cli can validate both schema and multiple instances.

It support both json and yaml files

exit code is: 
- `1` if command line arguments are invalid.
- `2` if there are errors
