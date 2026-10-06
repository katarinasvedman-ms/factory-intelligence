# Edge deployment integration

Foundry Local on Azure Local is preview functionality. Do not apply copied manifests
without checking the current environment and official documentation.

This application needs two separately configured URLs:

- `EDGE_BASE_URL`: the deployed model's OpenAI-compatible data-plane URL.
- `EDGE_CONTROL_PLANE_URL`: the Foundry Local control-plane URL used by
  the `factory-eval` Rust binary.

Configure the model deployment in the target Arc-enabled Kubernetes environment,
then set `EDGE_MODEL`, the selected authentication mode, and the corresponding
credential environment variable. Keep full endpoint URLs and credentials outside
source control.

The example configuration assumes `/v1/chat/completions` and `/v1/models`. Override
`chat_path`, `health_path`, authentication mode, and header names in `config/demo.yaml`
when the verified environment differs.
