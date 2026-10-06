# Demo application deployment

Build the application container from the repository root:

```powershell
docker build -t factory-intelligence-demo .
```

Supply `config/demo.yaml` and provider credentials at runtime. Do not bake credentials
or customer endpoint URLs into the image.
