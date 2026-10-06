using './main.bicep'

param location = readEnvironmentVariable('FACTORY_LOCATION')
param namePrefix = readEnvironmentVariable('FACTORY_NAME_PREFIX')
param sshPublicKey = readEnvironmentVariable('AKS_SSH_PUBLIC_KEY')
param adminUsername = 'azureuser'
param deployGpuPool = true
param systemVmSize = 'Standard_D4s_v5'
param cpuVmSize = 'Standard_D8s_v5'
param gpuVmSize = 'Standard_NC24ads_A100_v4'
param gpuNodeCount = 3
param cpuNodeCount = 3
