# A minimal Tarn test file

Create a file named `tests/health.tarn.yaml`:

```yaml
version: "1"
name: "Health check"

tests:
  service_is_up:
    steps:
      - name: GET /health
        request:
          method: GET
          url: "https://httpbin.org/status/200"
        assert:
          status: 200
```

Save the file. The extension will index it automatically. You'll see it appear in the Test Explorer under **Tarn**.
