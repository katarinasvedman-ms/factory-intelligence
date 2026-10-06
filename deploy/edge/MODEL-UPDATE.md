# Controlled edge model update

The application does not assume that the model reference on an existing deployment
can be patched. The current environment must determine whether the supported update
operation is replacement, full deployment update, or creation of a second deployment.

Use this rehearsed sequence:

1. Record the current deployment name, resolved model identifier, endpoint alias,
   readiness state, and evaluation result.
2. Prepare the new model/deployment using the control-plane method verified for the
   target Foundry Local on Azure Local version.
3. Wait until the new deployment reports ready.
4. Update only the edge provider configuration (`EDGE_BASE_URL` and `EDGE_MODEL`, or
   the corresponding secret/configuration binding).
5. Restart or reload the demo application configuration.
6. Run:

   ```powershell
   cargo run --bin factory-smoke -- --target edge --scenario cross-machine-correlation
   ```

7. Run evaluation when the environment supports it.
8. Record the before/after identifiers and results.

Do not demonstrate rollback until the old deployment remains available and switching
back has been tested in the presentation environment.
