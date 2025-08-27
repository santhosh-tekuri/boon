# Changelog

## [Unreleased]

### Bug Fixes
- validator: ensure `uneval` state is propagated when `$ref` validation fails

## [0.6.1] - 2025-01-07

### Bug Fixes
- fix: FileLoader should not be used in wasm

## [0.6.0] - 2024-05-30

### Braking Changes
- loader: Allow to replace entirely

### Bug Fixes
- seperate doc loading from root creation
- validator: if contentEncoding fails, skip contentMediaType
- loader: should load latest from metaschemas dir
- fix: hash for json numbers with zero fractions
- fix: resources/anchors in non-std schema loc not supported

### Changes
- boon binary artificats under github release
- boon binary `--cacert` option
- boon binary `--insecure` flag

## [0.5.3] - 2024-01-27

### Changes
- updated dependencies

## [0.5.2] - 2024-01-27

### Bug Fixes

- Error message for failed const validation is wrong

## [0.5.1] - 2023-07-13

### Changes

- WASM compatibility
- minor performance improvements

## [0.5.0] - 2023-03-29

### Breaking Changes
- chages to error api

### Performance
- minor improvements in validation

## [0.4.0] - 2023-03-24

### Breaking Changes
- chages to error api

### Fixed
- Compler.add_resource should not check file exists

### Added
- implement `contentSchema` keyword
- ECMA-262 regex compatibility
- add example_custom_content_encoding
- add example_custom_content_media_type

### Performance
- significant improvement in validation

## [0.3.1] - 2023-03-07

### Added
- add example_from_yaml_files
- cli: support yaml files

### Fixed
- ensure fragment decoded before use
- $dynamicRef w/o anchor is same as $ref