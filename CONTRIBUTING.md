# Contributing

## Development

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test --all
cargo test -p tarn --test conformance_test
bash scripts/ci/smoke.sh
```

## Before Opening a PR

- keep changes focused;
- add tests for behavior changes;
- update docs when CLI behavior or output changes;
- keep the JSON contract backward-compatible within the same schema version.
- keep examples and conformance fixtures aligned with the shipped DSL.

## Test Expectations

- new behavior needs tests;
- bug fixes need a regression test;
- prefer realistic integration coverage over shallow unit coverage when possible.
- if the change affects canonical examples or formatter behavior, update the conformance suite too.

## Release-Sensitive Areas

Be careful when changing:

- `tarn run --format json`
- env/config resolution
- install/update scripts
- GitHub Action behavior
- MCP tool behavior
