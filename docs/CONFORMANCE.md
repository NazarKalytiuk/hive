# Tarn Conformance Suite

Tarn ships a small public conformance suite under [`tarn/tests/conformance/manifest.json`](../tarn/tests/conformance/manifest.json).

Goals:

- keep core `.tarn.yaml` parsing behavior stable across releases
- make example coverage part of the public compatibility surface
- expose a concrete suite that downstream packagers and contributors can run locally

## What it covers

- parsing of canonical examples under `examples/`
- formatting round-trip for representative fixtures
- init/demo-server authored flows that represent the supported DSL surface

## How to run it

```bash
cargo test -p tarn --test conformance_test
```

The suite is also executed in CI as a dedicated compatibility/conformance job and backs the public compatibility badge shown in the repository surface.
