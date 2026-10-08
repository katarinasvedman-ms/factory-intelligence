# Azure evaluation infrastructure

This infrastructure deploys into the existing `factory-intelligence` resource group
and creates the Azure foundation for the easy-demo topology:

```text
Windows laptop = robot / fast path
Azure-hosted AKS connected to Azure Arc = evaluation factory cluster / slow path
Optional cloud = expert path
```

This is the Microsoft evaluation quickstart topology. It is not AKS Arc on Azure
Local and must not be presented as the production factory topology. The intended
production target remains AKS Arc running on Azure Local.

## What Bicep deploys

- virtual network and AKS subnet;
- user-assigned AKS identity with Network Contributor on the AKS subnet;
- AKS system, CPU, and optional GPU pools;
- Log Analytics;
- Azure Container Registry;
- premium Azure Files NFS share;
- private endpoint and private DNS for NFS.

## What the bootstrap script deploys

These components intentionally remain outside Bicep because they require live
cluster onboarding, preview extension workflows, Helm, and Entra application IDs:

- Azure Arc connection;
- NVIDIA GPU operator;
- certificate and trust-manager extension;
- Foundry Local inference operator;
- Foundry Local model deployment;
- Agentic Retrieval extension in `combined` mode;
- Azure RBAC assignments available after extension identities exist.

The `FoundryInferenceAccess` Entra app-role assignment must be completed separately
because it is a Microsoft Graph operation and requires appropriate directory
permissions.

## Factory advisory messaging

The machine-to-cluster advisory path is deployed separately from the Agentic
Retrieval extension:

```powershell
.\infra\deploy-advisory-messaging.ps1 `
  -AcrName "<container-registry-name>" `
  -ForceBuild
```

The script:

- builds `factory-advisory-worker` in ACR when requested or when its tag is absent;
- deploys a persistent Mosquitto broker in namespace `factory-messaging`;
- deploys the durable advisory worker on the factory CPU node pool;
- creates MQTT and Agentic Retrieval secrets;
- discovers the external broker address;
- writes the active non-source-controlled machine settings to `.env.local`.

Verify:

```powershell
kubectl get pods -n factory-messaging
kubectl logs deployment/factory-advisory-worker -n factory-messaging --tail=50
kubectl logs statefulset/factory-mqtt -n factory-messaging --tail=50
```

Rerun the script without `-ForceBuild` to refresh the time-limited Agentic Retrieval
token. The script preserves the MQTT password from `.env.local`. If the Kubernetes
MQTT Secret is changed independently, restart the broker so its generated password
file matches:

```powershell
kubectl rollout restart statefulset/factory-mqtt -n factory-messaging
kubectl rollout status statefulset/factory-mqtt -n factory-messaging --timeout=10m
```

The evaluation service currently exposes MQTT port 1883 through an Azure load
balancer. Production factory deployments must use private connectivity and TLS or
mTLS, and should replace the delegated user token with workload identity or another
renewable service identity.

## Prerequisites

- Azure CLI signed in to the target subscription;
- Contributor access at subscription scope;
- `kubectl`;
- Helm;
- OpenSSH client;
- sufficient regional quota for the selected CPU and GPU SKUs;
- `Microsoft.Network/AllowBringYourOwnPublicIpAddress` registered for the subscription;
- Agentic Retrieval and Foundry Local preview access;
- two Entra app registrations:
  - Agentic Retrieval application;
  - Foundry Local application.
- DNS domain and certificate plan.

## 1. Validate quota

The default GPU size is expensive and frequently quota-constrained:

```powershell
az feature register `
  --namespace Microsoft.Network `
  --name AllowBringYourOwnPublicIpAddress
az provider register --namespace Microsoft.Network --wait

az vm list-usage --location swedencentral --output table
az vm list-skus --location swedencentral --size Standard_NC24ads_A100_v4 --all --output table
```

The default three-node A100 pool needs 72 vCPUs in
`StandardNCADSA100v4Family`: two GPUs for Agentic Retrieval embeddings and one GPU
for the Foundry Local language model. If its quota is zero, request the limit through
Azure Quotas before enabling the GPU pool:

```powershell
$subscriptionId = az account show --query id --output tsv
$scope = "/subscriptions/$subscriptionId/providers/Microsoft.Compute/locations/swedencentral"

az quota create `
  --resource-name StandardNCADSA100v4Family `
  --scope $scope `
  --limit-object value=72 `
  --resource-type dedicated
```

Do not enable the GPU pool until the request is approved and SKU availability is
confirmed.

## 2. Deploy the Azure foundation

```powershell
.\infra\deploy.ps1 `
  -ResourceGroup factory-intelligence `
  -Location swedencentral `
  -NamePrefix factoryintel
```

To validate the inexpensive base resources before GPU quota is available:

```powershell
.\infra\deploy.ps1 `
  -ResourceGroup factory-intelligence `
  -Location swedencentral `
  -NamePrefix factoryintel `
  -SkipGpuPool
```

Agentic Retrieval and the Foundry Local model cannot become operational without the
required GPU capacity.

## 3. Create the Entra applications

The default demo hostname uses the reserved `.test` domain and is resolved locally
through the Windows hosts file after ingress receives an IP:

```powershell
.\infra\create-entra-apps.ps1
```

Save the returned `EdgeRagClientId` and `FoundryClientId`.

After the Agentic Retrieval extension creates its managed identity, rerun the script
with that service principal's object ID so the runtime can authenticate to the MCP
server without relying on a delegated user token:

```powershell
.\infra\create-entra-apps.ps1 `
  -AgenticRuntimePrincipalId "<agentic-runtime-service-principal-object-id>"
```

This grants only the `EdgeRAGEndUser` application role. The script is idempotent.

## 4. Bootstrap Arc and preview extensions

Install Helm first, then:

```powershell
.\infra\bootstrap-edge.ps1 `
  -ResourceGroup factory-intelligence `
  -ClusterName factoryintel-aks `
  -Location swedencentral `
  -DomainName arcrag.factory-intelligence.test `
  -EdgeRagClientId "<agentic-retrieval-app-client-id>" `
  -FoundryClientId "<foundry-local-app-client-id>"
```

While GPU quota is pending, deploy only the Arc connection and certificate
foundation:

```powershell
.\infra\bootstrap-edge.ps1 `
  -ResourceGroup factory-intelligence `
  -ClusterName factoryintel-aks `
  -Location swedencentral `
  -DomainName arcrag.factory-intelligence.test `
  -EdgeRagClientId "<agentic-retrieval-app-client-id>" `
  -FoundryClientId "<foundry-local-app-client-id>" `
  -PlatformOnly
```

The script is intentionally idempotent where the underlying CLI supports repeated
create operations. Review extension status before rerunning after a partial failure.

### CPU-only Agentic mode with keyless cloud Foundry

The CPU-only design keeps Fast inference on the machine and runs the intended Slow
agentic layer on AKS. A bridge inside AKS uses Workload Identity to invoke
`factory-slow-gpt5-mini` while the Foundry resource keeps
`disableLocalAuth=true`.

```powershell
.\infra\bootstrap-agentic-cloud.ps1 `
  -ResourceGroup "<aks-resource-group>" `
  -ClusterName "<aks-cluster-name>" `
  -FoundryResourceGroup "<foundry-resource-group>" `
  -FoundryAccountName "<foundry-account-name>" `
  -FoundryDeploymentName "<slow-model-deployment>" `
  -AcrResourceGroup "<registry-resource-group>" `
  -AcrName "<registry-name>" `
  -BridgeIdentityName "<bridge-identity-name>" `
  -EdgeRagClientId "<entra-application-client-id>" `
  -AgenticPublicIpName "<agentic-public-ip-name>"
```

The script:

- configures the EdgeRAG delegated `access_as_user` scope;
- creates a user-assigned managed identity and federated credential;
- grants only `Cognitive Services OpenAI User` on the Foundry resource;
- deploys an internal OpenAI-compatible bridge on the CPU node pool;
- restricts bridge ingress to the `arc-rag` namespace;
- installs Agentic Retrieval in `agentic` mode when the extension artifact is
  available to the subscription.

The Arc extension requires a Kubernetes secret named `byom-api-key`. In this
design, that secret authenticates only to the internal bridge; it is not a Foundry
or Azure API key. The bridge acquires a Microsoft Entra token through AKS Workload
Identity.

The validated evaluation deployment uses East US 2 and Kubernetes 1.34.11, where
the preview catalog exposes `microsoft.arc.rag` 0.9.3. Sweden Central with
Kubernetes 1.35.7 did not expose an installable artifact.

```powershell
.\infra\bootstrap-agentic-cloud.ps1 `
  -ResourceGroup "<aks-resource-group>" `
  -ClusterName "<aks-cluster-name>" `
  -FoundryResourceGroup "<foundry-resource-group>" `
  -FoundryAccountName "<foundry-account-name>" `
  -FoundryDeploymentName "<slow-model-deployment>" `
  -AcrResourceGroup "<registry-resource-group>" `
  -AcrName "<registry-name>" `
  -BridgeIdentityName "<bridge-identity-name>" `
  -EdgeRagClientId "<entra-application-client-id>" `
  -AgenticPublicIpName "<agentic-public-ip-name>"
.\infra\deploy-manual-mcp.ps1 `
  -ResourceGroup "<aks-resource-group>" `
  -ClusterName "<aks-cluster-name>" `
  -AcrResourceGroup "<registry-resource-group>" `
  -AcrName "<registry-name>" `
  -Audience "<entra-application-client-id>" `
  -McpPublicIpName "<mcp-public-ip-name>"
.\infra\configure-agentic-mcp.ps1 `
  -AgenticDomain "<agentic-domain>" `
  -McpDomain "<mcp-domain>" `
  -ClientId "<entra-application-client-id>"
.\run-aks-agentic-demo.ps1 `
  -AgenticDomain "<agentic-domain>" `
  -ClientId "<entra-application-client-id>" `
  -FoundryBaseUrl "https://<foundry-resource>.openai.azure.com/openai/v1" `
  -FoundryExpertModel "<expert-model-deployment>"
```

## 5. Deploy the Rust application

Build and push:

```powershell
az acr login --name "<registry-name>"
docker build -t "<registry-login-server>/factory-intelligence:demo" .
docker push "<registry-login-server>/factory-intelligence:demo"
```

Application Kubernetes manifests are added after the Agentic Retrieval query
contract and authentication behavior have been captured from the preview environment.

## 6. Publish and configure factory knowledge

Mount the NFS share from a management host that can reach its private endpoint, then
publish the synthetic manuals:

```powershell
.\infra\publish-knowledge.ps1 -NfsMountPath "<mounted-nfs-path>"
```

After the combined Agentic Retrieval extension is healthy, create the collection,
start ingestion, register the built-in indexed-source MCP server, and link the
knowledge source to the default knowledge base:

```powershell
.\infra\configure-agentic-layer.ps1 `
  -DomainName arcrag.factory-intelligence.test `
  -ClientId "<agentic-retrieval-app-client-id>" `
  -NfsServerPath "<nfs-server-ip>:/factory-maintenance-demo"
```

Save the returned `AgentId`. Run the application with:

```powershell
$env:EDGE_AGENTIC_BASE_URL = "https://arcrag.factory-intelligence.test"
$env:EDGE_AGENTIC_TOKEN = az account get-access-token `
  --resource "api://<agentic-retrieval-app-client-id>" `
  --query accessToken `
  --output tsv
$env:EDGE_AGENT_ID = "<returned-agent-id>"
$env:DEMO_CONFIG = "config\demo.agentic.yaml"
cargo run --release --bin factory-intelligence
```

## Cost control

Use a dedicated resource group and delete it when the demo window closes:

```powershell
az stack group delete `
  --resource-group factory-intelligence `
  --name factory-intelligence-demo `
  --action-on-unmanage deleteResources `
  --yes
```

The resource group is shared and must not be deleted as part of demo cleanup. Delete
only the resources created by this deployment. Do not present the Azure-hosted AKS
evaluation environment as an Azure Local production deployment.
