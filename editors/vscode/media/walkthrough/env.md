# Environments

Tarn resolves variables from a precedence chain. In order from lowest to highest priority:

1. The `env:` block inside each test file.
2. `tarn.env.yaml` — default environment.
3. `tarn.env.{name}.yaml` — named environment, selected via `--env`.
4. `tarn.env.local.yaml` — gitignored local overrides.
5. Shell environment variables.
6. `--var KEY=VALUE` from the command line.

To create a named environment, add `tarn.env.staging.yaml` at the workspace root:

```yaml
base_url: "https://staging.example.com"
admin_email: "ops@example.com"
```

Then pick **staging** from the Tarn status bar at the bottom of the window. Every subsequent run will pass `--env staging`.
