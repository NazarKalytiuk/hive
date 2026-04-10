# Run your first test

You have three ways to run a test:

1. **Test Explorer** — open the flask icon on the Activity Bar and click the play icon next to any test, step, or file.
2. **CodeLens** — open a `.tarn.yaml` file and click the `Run` lens above any test or step.
3. **Command palette** — `Tarn: Run All Tests`, `Tarn: Run Current File`, or `Tarn: Dry Run Current File`.

Dry Run interpolates every template but does not send HTTP requests — useful to confirm that `{{ env.x }}` and `{{ capture.y }}` resolve correctly before hitting a live service.

Results land in the Test Explorer and in the Run History view at the bottom of the Tarn Activity Bar container.
