targetScope = 'resourceGroup'

@description('Azure region for the evaluation environment.')
param location string

@description('Short prefix used for globally unique resource names.')
@minLength(3)
@maxLength(20)
param namePrefix string = 'factorydemo'

@description('SSH public key used for AKS Linux nodes.')
param sshPublicKey string

@description('Linux administrator username for AKS nodes.')
param adminUsername string = 'azureuser'

@description('Deploy the expensive GPU node pool. Set false only for base-infrastructure validation.')
param deployGpuPool bool = true

@description('VM size for the AKS system node pool.')
param systemVmSize string = 'Standard_D4s_v5'

@description('VM size for the Agentic Retrieval CPU node pool.')
param cpuVmSize string = 'Standard_D8s_v5'

@description('GPU VM size used by Agentic Retrieval and Foundry Local.')
param gpuVmSize string = 'Standard_NC24ads_A100_v4'

@description('Number of GPU worker nodes.')
@minValue(0)
param gpuNodeCount int = 3

@description('Number of CPU worker nodes.')
@minValue(1)
param cpuNodeCount int = 3

@description('Tags applied to all Azure resources.')
param tags object = {
  workload: 'factory-intelligence'
  environment: 'demo'
  managedBy: 'bicep'
  preview: 'agentic-retrieval'
}

module platform 'modules/platform.bicep' = {
  name: 'factory-intelligence-platform'
  params: {
    location: location
    namePrefix: namePrefix
    sshPublicKey: sshPublicKey
    adminUsername: adminUsername
    deployGpuPool: deployGpuPool
    systemVmSize: systemVmSize
    cpuVmSize: cpuVmSize
    gpuVmSize: gpuVmSize
    gpuNodeCount: gpuNodeCount
    cpuNodeCount: cpuNodeCount
    tags: tags
  }
}

output resourceGroupName string = resourceGroup().name
output clusterName string = platform.outputs.clusterName
output containerRegistryName string = platform.outputs.containerRegistryName
output containerRegistryLoginServer string = platform.outputs.containerRegistryLoginServer
output nfsStorageAccountName string = platform.outputs.nfsStorageAccountName
output nfsShareName string = platform.outputs.nfsShareName
output nfsServer string = platform.outputs.nfsServer
output nfsExportPath string = platform.outputs.nfsExportPath
output logAnalyticsWorkspaceName string = platform.outputs.logAnalyticsWorkspaceName
output aksIdentityName string = platform.outputs.aksIdentityName
