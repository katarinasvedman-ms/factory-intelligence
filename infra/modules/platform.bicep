targetScope = 'resourceGroup'

param location string
@minLength(3)
@maxLength(20)
param namePrefix string
param sshPublicKey string
param adminUsername string
param deployGpuPool bool
param systemVmSize string
param cpuVmSize string
param gpuVmSize string
param gpuNodeCount int
param cpuNodeCount int
param tags object

var suffix = uniqueString(subscription().subscriptionId, resourceGroup().id, namePrefix)
var clusterName = '${namePrefix}-aks'
var registryName = 'fi${suffix}acr'
var storageName = 'fi${suffix}nfs'
var logAnalyticsName = '${namePrefix}-logs'
var virtualNetworkName = '${namePrefix}-vnet'
var aksSubnetName = 'aks'
var privateEndpointSubnetName = 'private-endpoints'
var nfsShareName = 'factory-knowledge'

resource logAnalytics 'Microsoft.OperationalInsights/workspaces@2023-09-01' = {
  name: logAnalyticsName
  location: location
  tags: tags
  properties: {
    retentionInDays: 30
    features: {
      enableLogAccessUsingOnlyResourcePermissions: true
    }
  }
}

resource virtualNetwork 'Microsoft.Network/virtualNetworks@2024-05-01' = {
  name: virtualNetworkName
  location: location
  tags: tags
  properties: {
    addressSpace: {
      addressPrefixes: [
        '10.40.0.0/16'
      ]
    }
    subnets: [
      {
        name: aksSubnetName
        properties: {
          addressPrefix: '10.40.0.0/20'
        }
      }
      {
        name: privateEndpointSubnetName
        properties: {
          addressPrefix: '10.40.16.0/24'
          privateEndpointNetworkPolicies: 'Disabled'
        }
      }
    ]
  }
}

resource aksSubnet 'Microsoft.Network/virtualNetworks/subnets@2024-05-01' existing = {
  parent: virtualNetwork
  name: aksSubnetName
}

resource privateEndpointSubnet 'Microsoft.Network/virtualNetworks/subnets@2024-05-01' existing = {
  parent: virtualNetwork
  name: privateEndpointSubnetName
}

resource aksIdentity 'Microsoft.ManagedIdentity/userAssignedIdentities@2023-01-31' = {
  name: '${namePrefix}-aks-identity'
  location: location
  tags: tags
}

resource aksSubnetNetworkContributor 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(aksSubnet.id, aksIdentity.id, 'network-contributor')
  scope: aksSubnet
  properties: {
    principalId: aksIdentity.properties.principalId
    principalType: 'ServicePrincipal'
    roleDefinitionId: subscriptionResourceId(
      'Microsoft.Authorization/roleDefinitions',
      '4d97b98b-1d4f-4787-a291-c67834d212e7'
    )
  }
}

resource registry 'Microsoft.ContainerRegistry/registries@2023-11-01-preview' = {
  name: registryName
  location: location
  tags: tags
  sku: {
    name: 'Basic'
  }
  properties: {
    adminUserEnabled: false
    anonymousPullEnabled: false
    dataEndpointEnabled: false
    publicNetworkAccess: 'Enabled'
    policies: {
      exportPolicy: {
        status: 'enabled'
      }
      quarantinePolicy: {
        status: 'disabled'
      }
      retentionPolicy: {
        days: 7
        status: 'disabled'
      }
      trustPolicy: {
        status: 'disabled'
        type: 'Notary'
      }
    }
  }
}

resource storage 'Microsoft.Storage/storageAccounts@2023-05-01' = {
  name: storageName
  location: location
  tags: tags
  kind: 'FileStorage'
  sku: {
    name: 'Premium_LRS'
  }
  properties: {
    allowBlobPublicAccess: false
    allowSharedKeyAccess: true
    minimumTlsVersion: 'TLS1_2'
    publicNetworkAccess: 'Disabled'
    supportsHttpsTrafficOnly: false
  }
}

resource fileService 'Microsoft.Storage/storageAccounts/fileServices@2023-05-01' = {
  parent: storage
  name: 'default'
  properties: {
    protocolSettings: {
      smb: {}
    }
  }
}

resource nfsShare 'Microsoft.Storage/storageAccounts/fileServices/shares@2023-05-01' = {
  parent: fileService
  name: nfsShareName
  properties: {
    accessTier: 'Premium'
    enabledProtocols: 'NFS'
    rootSquash: 'NoRootSquash'
    shareQuota: 100
  }
}

resource filePrivateDnsZone 'Microsoft.Network/privateDnsZones@2024-06-01' = {
  name: 'privatelink.file.${environment().suffixes.storage}'
  location: 'global'
  tags: tags
}

resource filePrivateDnsLink 'Microsoft.Network/privateDnsZones/virtualNetworkLinks@2024-06-01' = {
  parent: filePrivateDnsZone
  name: '${namePrefix}-file-vnet-link'
  location: 'global'
  properties: {
    registrationEnabled: false
    virtualNetwork: {
      id: virtualNetwork.id
    }
  }
}

resource storagePrivateEndpoint 'Microsoft.Network/privateEndpoints@2024-05-01' = {
  name: '${namePrefix}-file-pe'
  location: location
  tags: tags
  properties: {
    subnet: {
      id: privateEndpointSubnet.id
    }
    privateLinkServiceConnections: [
      {
        name: 'file'
        properties: {
          privateLinkServiceId: storage.id
          groupIds: [
            'file'
          ]
        }
      }
    ]
  }
}

resource storagePrivateDnsGroup 'Microsoft.Network/privateEndpoints/privateDnsZoneGroups@2024-05-01' = {
  parent: storagePrivateEndpoint
  name: 'default'
  properties: {
    privateDnsZoneConfigs: [
      {
        name: 'file'
        properties: {
          privateDnsZoneId: filePrivateDnsZone.id
        }
      }
    ]
  }
}

resource cluster 'Microsoft.ContainerService/managedClusters@2024-10-01' = {
  name: clusterName
  location: location
  tags: tags
  identity: {
    type: 'UserAssigned'
    userAssignedIdentities: {
      '${aksIdentity.id}': {}
    }
  }
  sku: {
    name: 'Base'
    tier: 'Free'
  }
  properties: {
    dnsPrefix: clusterName
    enableRBAC: true
    disableLocalAccounts: false
    linuxProfile: {
      adminUsername: adminUsername
      ssh: {
        publicKeys: [
          {
            keyData: sshPublicKey
          }
        ]
      }
    }
    aadProfile: {
      enableAzureRBAC: true
      managed: true
      tenantID: tenant().tenantId
    }
    agentPoolProfiles: [
      {
        name: 'system'
        count: 2
        vmSize: systemVmSize
        osType: 'Linux'
        osSKU: 'AzureLinux'
        mode: 'System'
        type: 'VirtualMachineScaleSets'
        vnetSubnetID: aksSubnet.id
        maxPods: 50
        enableAutoScaling: true
        minCount: 2
        maxCount: 3
        nodeLabels: {
          workload: 'system'
        }
      }
    ]
    apiServerAccessProfile: {
      enablePrivateCluster: false
    }
    autoUpgradeProfile: {
      nodeOSUpgradeChannel: 'NodeImage'
      upgradeChannel: 'patch'
    }
    networkProfile: {
      loadBalancerSku: 'standard'
      networkDataplane: 'cilium'
      networkPlugin: 'azure'
      networkPluginMode: 'overlay'
      networkPolicy: 'cilium'
      outboundType: 'loadBalancer'
      serviceCidr: '10.42.0.0/16'
      dnsServiceIP: '10.42.0.10'
    }
    oidcIssuerProfile: {
      enabled: true
    }
    securityProfile: {
      workloadIdentity: {
        enabled: true
      }
    }
    addonProfiles: {
      omsagent: {
        enabled: true
        config: {
          logAnalyticsWorkspaceResourceID: logAnalytics.id
          useAADAuth: 'true'
        }
      }
    }
  }
  dependsOn: [
    aksSubnetNetworkContributor
  ]
}

resource cpuPool 'Microsoft.ContainerService/managedClusters/agentPools@2024-10-01' = {
  parent: cluster
  name: 'cpu'
  properties: {
    count: cpuNodeCount
    vmSize: cpuVmSize
    osType: 'Linux'
    osSKU: 'AzureLinux'
    mode: 'User'
    type: 'VirtualMachineScaleSets'
    vnetSubnetID: aksSubnet.id
    maxPods: 50
    enableAutoScaling: true
    minCount: cpuNodeCount
    maxCount: cpuNodeCount
    nodeLabels: {
      workload: 'factory-cpu'
    }
  }
}

resource gpuPool 'Microsoft.ContainerService/managedClusters/agentPools@2024-10-01' = if (deployGpuPool) {
  parent: cluster
  name: 'gpu'
  properties: {
    count: gpuNodeCount
    vmSize: gpuVmSize
    osType: 'Linux'
    osSKU: 'Ubuntu'
    mode: 'User'
    type: 'VirtualMachineScaleSets'
    vnetSubnetID: aksSubnet.id
    maxPods: 30
    enableAutoScaling: true
    minCount: gpuNodeCount
    maxCount: gpuNodeCount
    nodeLabels: {
      workload: 'factory-gpu'
    }
    nodeTaints: [
      'sku=gpu:NoSchedule'
    ]
  }
}

resource acrPull 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(registry.id, cluster.id, 'acr-pull')
  scope: registry
  properties: {
    principalId: cluster.properties.identityProfile.kubeletidentity.objectId
    principalType: 'ServicePrincipal'
    roleDefinitionId: subscriptionResourceId(
      'Microsoft.Authorization/roleDefinitions',
      '7f951dda-4ed3-4680-a7ca-43fe172d538d'
    )
  }
}

output clusterName string = cluster.name
output containerRegistryName string = registry.name
output containerRegistryLoginServer string = registry.properties.loginServer
output nfsStorageAccountName string = storage.name
output nfsShareName string = nfsShare.name
output nfsServer string = '${storage.name}.file.${environment().suffixes.storage}'
output nfsExportPath string = '/${storage.name}/${nfsShare.name}'
output logAnalyticsWorkspaceName string = logAnalytics.name
output aksIdentityName string = aksIdentity.name
