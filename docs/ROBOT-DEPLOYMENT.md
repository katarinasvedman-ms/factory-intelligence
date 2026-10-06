# Running locally near a robot

## Recommended placement

Run the Rust application and Foundry Local on the robot-connected industrial PC,
embedded GPU computer, or edge controller that has a supported general-purpose
operating system. Do not place this demo on a PLC, safety controller, or constrained
microcontroller, and do not put inference in a real-time control loop.

The application communicates with Foundry Local over loopback using its
OpenAI-compatible REST API:

```text
Robot sensors / synthetic event
        |
        v
Rust Factory Intelligence service
        |
        v
http://127.0.0.1:<dynamic-port>/v1/chat/completions
        |
        v
Foundry Local model runtime
```

## Device qualification

Validate each target device before deployment:

- supported x64 or ARM64 operating system for the chosen Foundry Local package;
- enough RAM and storage for the selected model and model cache;
- suitable CPU, GPU, or NPU execution provider and drivers;
- measured latency under the robot application's actual workload;
- local endpoint discovery rather than a hard-coded dynamic port;
- offline model pre-download when disconnected operation is required;
- thermal, power, and startup behavior suitable for the industrial PC;
- process supervision and recovery after reboot.

The Rust service itself is small compared with model files and inference-runtime
memory. Model selection and quantization determine whether a specific robot computer
is practical.

## Safety boundary

- Model output remains advisory and is never translated directly into commands.
- The current reduce-speed connector is simulated and must not write to PLC, SCADA,
  MES, robot motion, or safety APIs.
- A deterministic application rule—not the model—decides when factory-level
  escalation occurs.
- Reduce-speed requests must reference successful inference evidence, remain within
  the application-defined bound, and receive explicit operator approval.
- Any future physical connector must use authorized operational software to enforce
  machine state validation, safety interlocks, access control, outcome verification,
  and recovery independently of the model.

## Foundry Local integration choices

1. **Sidecar/service mode (current implementation):** run Foundry Local separately and
   configure `DEVICE_BASE_URL` and `DEVICE_MODEL`. This keeps the provider contract
   identical across device, edge, and cloud.
2. **Embedded Rust SDK:** initialize `foundry-local-sdk` in the device process, select
   and load a local model, then either call its native API or start its local REST
   server. Use this only after validating packaging and execution-provider behavior
   on the exact robot computer.

Official references:

- https://learn.microsoft.com/azure/foundry-local/reference/reference-sdk-current
- https://learn.microsoft.com/azure/foundry-local/reference/reference-rest
- https://learn.microsoft.com/azure/foundry-local/how-to/how-to-integrate-with-inference-sdks
