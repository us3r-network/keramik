use std::collections::{BTreeMap, HashMap};

use k8s_openapi::{
    api::{
        apps::v1::{RollingUpdateStatefulSetStrategy, StatefulSetSpec, StatefulSetUpdateStrategy},
        core::v1::{
            ConfigMapVolumeSource, Container, ContainerPort, EmptyDirVolumeSource, EnvVar,
            EnvVarSource, HTTPGetAction, PersistentVolumeClaim, PersistentVolumeClaimSpec,
            PersistentVolumeClaimVolumeSource, PodSecurityContext, PodSpec, PodTemplateSpec, Probe,
            ResourceRequirements, SecretKeySelector, ServicePort, ServiceSpec, Volume, VolumeMount,
        },
    },
    apimachinery::pkg::{
        api::resource::Quantity, apis::meta::v1::LabelSelector, util::intstr::IntOrString,
    },
};
use kube::core::ObjectMeta;

use crate::labels::{managed_labels, selector_labels};
use crate::network::{
    controller::{
        CAS_SERVICE_NAME, CERAMIC_APP, CERAMIC_LOCAL_NETWORK_TYPE, GANACHE_SERVICE_NAME,
        INIT_CONFIG_MAP_NAME,
    },
    datadog::DataDogConfig,
    resource_limits::ResourceLimitsConfig,
    CeramicSpec, GoIpfsSpec, IpfsSpec, NetworkSpec, RustIpfsSpec,
};

use crate::network::controller::{CERAMIC_SERVICE_API_PORT, CERAMIC_SERVICE_IPFS_PORT};

use super::controller::{CERAMIC_POSTGRES_APP, CERAMIC_POSTGRES_SERVICE_NAME, DB_TYPE_POSTGRES};

const IPFS_CONTAINER_NAME: &str = "ipfs";
const IPFS_DATA_PV_CLAIM: &str = "ipfs-data";

pub fn config_maps(
    info: &CeramicInfo,
    config: &CeramicConfig,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut config_maps = BTreeMap::new();
    if config.init_config_map == INIT_CONFIG_MAP_NAME {
        config_maps.insert(INIT_CONFIG_MAP_NAME.to_owned(),
            BTreeMap::from_iter(vec![
             ("ceramic-init.sh".to_owned(),
r#"#!/bin/bash

set -eo pipefail

export CERAMIC_ADMIN_DID=$(composedb did:from-private-key ${CERAMIC_ADMIN_PRIVATE_KEY})

CERAMIC_ADMIN_DID=$CERAMIC_ADMIN_DID envsubst < /ceramic-init/daemon-config.json > /config/daemon-config.json
"#.to_owned()),

("daemon-config.json".to_owned(),
r#"{
    "anchor": {
        "auth-method": "did"
    },
    "http-api": {
        "cors-allowed-origins": [
            "${CERAMIC_CORS_ALLOWED_ORIGINS}"
        ],
        "admin-dids": [
            "${CERAMIC_ADMIN_DID}"
        ]
    },
    "ipfs": {
        "mode": "remote",
        "host": "${CERAMIC_IPFS_HOST}"
    },
    "logger": {
        "log-level": ${CERAMIC_LOG_LEVEL},
        "log-to-files": false
    },
    "metrics": {
        "metrics-exporter-enabled": false,
        "prometheus-exporter-enabled": true,
        "prometheus-exporter-port": 9464
    },
    "network": {
        "name": "${CERAMIC_NETWORK}"
    },
    "node": {
        "privateSeedUrl": "inplace:ed25519#${CERAMIC_ADMIN_PRIVATE_KEY}"
    },
    "state-store": {
        "mode": "fs",
        "local-directory": "${CERAMIC_STATE_STORE_PATH}"
    },
    "indexing": {
        "db": "${DB_CONNECTION_STRING}",
        "allow-queries-before-historical-sync": true,
        "disable-composedb": false,
        "enable-historical-sync": ${ENABLE_HISTORICAL_SYNC}
    }
}"#.to_owned()),
]));
    }
    config_maps.append(&mut config.ipfs.config_maps(info));
    config_maps
}

pub fn service_spec() -> ServiceSpec {
    ServiceSpec {
        ports: Some(vec![
            ServicePort {
                port: CERAMIC_SERVICE_API_PORT,
                name: Some("api".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..Default::default()
            },
            ServicePort {
                port: CERAMIC_SERVICE_IPFS_PORT,
                name: Some("ipfs".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..Default::default()
            },
            ServicePort {
                port: 4001,
                name: Some("swarm-tcp".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..Default::default()
            },
        ]),
        selector: selector_labels(CERAMIC_APP),
        type_: Some("LoadBalancer".to_owned()),
        ..Default::default()
    }
}

pub struct CeramicConfig {
    pub weight: i32,
    pub init_config_map: String,
    pub image: String,
    pub image_pull_policy: String,
    pub ipfs: IpfsConfig,
    pub resource_limits: ResourceLimitsConfig,
    pub db_type: String,
    pub postgres: CeramicPostgres,
    pub enable_historical_sync: bool,
}

pub struct CeramicPostgres {
    /// Name of postgres db to use
    pub db_name: Option<String>,
    /// Name of postgres user to use
    pub user_name: Option<String>,
    /// Password for the postgres user
    pub password: Option<String>,
}

/// Bundles all relevant config for a ceramic spec.
pub struct CeramicBundle<'a> {
    pub info: CeramicInfo,
    pub config: &'a CeramicConfig,
    pub net_config: &'a NetworkConfig,
    pub datadog: &'a DataDogConfig,
}

// Contains top level config for the network
pub struct NetworkConfig {
    pub private_key_secret: Option<String>,
    pub network_type: String,
    pub pubsub_topic: String,
    pub eth_rpc_url: String,
    pub cas_api_url: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            private_key_secret: None,
            network_type: CERAMIC_LOCAL_NETWORK_TYPE.to_owned(),
            pubsub_topic: "/ceramic/local-keramik".to_owned(),
            eth_rpc_url: format!("http://{GANACHE_SERVICE_NAME}:8545"),
            cas_api_url: format!("http://{CAS_SERVICE_NAME}:8081"),
        }
    }
}

impl From<&NetworkSpec> for NetworkConfig {
    fn from(value: &NetworkSpec) -> Self {
        let default = NetworkConfig::default();
        Self {
            private_key_secret: value.private_key_secret.to_owned(),
            network_type: value
                .network_type
                .to_owned()
                .unwrap_or(default.network_type),
            pubsub_topic: value
                .pubsub_topic
                .to_owned()
                .unwrap_or(default.pubsub_topic),
            eth_rpc_url: value.eth_rpc_url.to_owned().unwrap_or(default.eth_rpc_url),
            cas_api_url: value.cas_api_url.to_owned().unwrap_or(default.cas_api_url),
        }
    }
}

/// Unique identifying information about this ceramic spec.
#[derive(Debug)]
pub struct CeramicInfo {
    pub replicas: i32,
    pub stateful_set: String,
    pub service: String,

    suffix: String,
}

impl CeramicInfo {
    pub fn new(suffix: &str, replicas: i32) -> Self {
        Self {
            replicas,
            suffix: suffix.to_owned(),
            stateful_set: format!("ceramic-{suffix}"),
            service: format!("ceramic-{suffix}"),
        }
    }
    /// Generate a new uninque name for this ceramic spec
    /// Generated name is deterministic for a given input name.
    pub fn new_name(&self, name: &str) -> String {
        format!("{name}-{}", self.suffix)
    }
    /// Determine the pod name
    pub fn pod_name(&self, peer: i32) -> String {
        format!("{}-{peer}", self.stateful_set)
    }
    /// Determine the IPFS RPC address of a Ceramic peer
    pub fn ipfs_rpc_addr(&self, ns: &str, peer: i32) -> String {
        format!(
            "http://{}-{peer}.{}.{ns}.svc.cluster.local:{CERAMIC_SERVICE_IPFS_PORT}",
            self.stateful_set, self.service
        )
    }
    /// Determine the Ceramic address of a Ceramic peer
    pub fn ceramic_addr(&self, ns: &str, peer: i32) -> String {
        format!(
            "http://{}-{peer}.{}.{ns}.svc.cluster.local:{CERAMIC_SERVICE_API_PORT}",
            self.stateful_set, self.service
        )
    }
}

pub enum IpfsConfig {
    Rust(RustIpfsConfig),
    Go(GoIpfsConfig),
}

impl Default for IpfsConfig {
    fn default() -> Self {
        Self::Rust(Default::default())
    }
}
impl IpfsConfig {
    fn config_maps(&self, info: &CeramicInfo) -> BTreeMap<String, BTreeMap<String, String>> {
        match self {
            IpfsConfig::Rust(_) => BTreeMap::new(),
            IpfsConfig::Go(config) => config.config_maps(info),
        }
    }
    fn container(&self, info: &CeramicInfo) -> Container {
        match self {
            IpfsConfig::Rust(config) => config.container(),
            IpfsConfig::Go(config) => config.container(info),
        }
    }
    fn volumes(&self, info: &CeramicInfo) -> Vec<Volume> {
        match self {
            IpfsConfig::Rust(_) => Vec::new(),
            IpfsConfig::Go(config) => config.volumes(info),
        }
    }
}

pub struct RustIpfsConfig {
    image: String,
    image_pull_policy: String,
    resource_limits: ResourceLimitsConfig,
    rust_log: String,
    env: Option<HashMap<String, String>>,
}

impl Default for RustIpfsConfig {
    fn default() -> Self {
        Self {
            image: "public.ecr.aws/r5b3e0r5/3box/ceramic-one:latest".to_owned(),
            image_pull_policy: "Always".to_owned(),
            resource_limits: ResourceLimitsConfig {
                cpu: Quantity("250m".to_owned()),
                memory: Quantity("512Mi".to_owned()),
                storage: Quantity("1Gi".to_owned()),
            },
            rust_log: "info,ceramic_one=debug,tracing_actix_web=debug,quinn_proto=error".to_owned(),
            env: None,
        }
    }
}
impl From<RustIpfsSpec> for RustIpfsConfig {
    fn from(value: RustIpfsSpec) -> Self {
        let default = RustIpfsConfig::default();
        Self {
            image: value.image.unwrap_or(default.image),
            image_pull_policy: value.image_pull_policy.unwrap_or(default.image_pull_policy),
            resource_limits: ResourceLimitsConfig::from_spec(
                value.resource_limits,
                default.resource_limits,
            ),
            rust_log: value.rust_log.unwrap_or(default.rust_log),
            env: value.env,
        }
    }
}

pub struct GoIpfsConfig {
    image: String,
    image_pull_policy: String,
    resource_limits: ResourceLimitsConfig,
    commands: Vec<String>,
}
impl Default for GoIpfsConfig {
    fn default() -> Self {
        Self {
            image: "ipfs/kubo:v0.19.1@sha256:c4527752a2130f55090be89ade8dde8f8a5328ec72570676b90f66e2cabf827d".to_owned(),
            image_pull_policy: "IfNotPresent".to_owned(),
            resource_limits: ResourceLimitsConfig {
                cpu: Quantity("1".to_owned()),
                memory: Quantity("2Gi".to_owned()),
                storage: Quantity("2Gi".to_owned()),
            },
            commands: vec![],
        }
    }
}
impl From<GoIpfsSpec> for GoIpfsConfig {
    fn from(value: GoIpfsSpec) -> Self {
        let default = GoIpfsConfig::default();
        Self {
            image: value.image.unwrap_or(default.image),
            image_pull_policy: value.image_pull_policy.unwrap_or(default.image_pull_policy),
            resource_limits: ResourceLimitsConfig::from_spec(
                value.resource_limits,
                default.resource_limits,
            ),
            commands: value.commands.unwrap_or(default.commands),
        }
    }
}

impl Default for CeramicConfig {
    fn default() -> Self {
        Self {
            weight: 1,
            init_config_map: INIT_CONFIG_MAP_NAME.to_owned(),
            image: "ceramicnetwork/composedb:latest".to_owned(),
            image_pull_policy: "Always".to_owned(),
            ipfs: IpfsConfig::default(),
            resource_limits: ResourceLimitsConfig {
                cpu: Quantity("1".to_owned()),
                memory: Quantity("1Gi".to_owned()),
                storage: Quantity("2Gi".to_owned()),
            },
            db_type: DB_TYPE_POSTGRES.to_owned(),
            postgres: CeramicPostgres {
                db_name: None,
                user_name: None,
                password: None,
            },
            enable_historical_sync: true,
        }
    }
}

pub struct CeramicConfigs(pub Vec<CeramicConfig>);

impl From<Vec<CeramicSpec>> for CeramicConfigs {
    fn from(value: Vec<CeramicSpec>) -> Self {
        if value.is_empty() {
            Self(vec![CeramicConfig::default()])
        } else {
            Self(value.into_iter().map(CeramicConfig::from).collect())
        }
    }
}

impl From<CeramicSpec> for CeramicConfig {
    fn from(value: CeramicSpec) -> Self {
        let default = Self::default();
        Self {
            weight: value.weight.unwrap_or(default.weight),
            init_config_map: value.init_config_map.unwrap_or(default.init_config_map),
            image: value.image.unwrap_or(default.image),
            image_pull_policy: value.image_pull_policy.unwrap_or(default.image_pull_policy),
            ipfs: value.ipfs.map(Into::into).unwrap_or(default.ipfs),
            resource_limits: ResourceLimitsConfig::from_spec(
                value.resource_limits,
                default.resource_limits,
            ),
            db_type: value.db_type.unwrap_or(default.db_type),
            postgres: CeramicPostgres {
                db_name: value.ceramic_postgres.clone().unwrap().db_name,
                user_name: value.ceramic_postgres.clone().unwrap().user_name,
                password: value.ceramic_postgres.clone().unwrap().password,
            },
            enable_historical_sync: value.enable_historical_sync.unwrap_or(default.enable_historical_sync),
        }
    }
}

impl From<IpfsSpec> for IpfsConfig {
    fn from(value: IpfsSpec) -> Self {
        match value {
            IpfsSpec::Rust(spec) => Self::Rust(spec.into()),
            IpfsSpec::Go(spec) => Self::Go(spec.into()),
        }
    }
}

impl RustIpfsConfig {
    fn container(&self) -> Container {
        let mut env = vec![
            EnvVar {
                name: "RUST_LOG".to_owned(),
                value: Some(self.rust_log.to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_BIND_ADDRESS".to_owned(),
                value: Some(format!("0.0.0.0:{CERAMIC_SERVICE_IPFS_PORT}")),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_METRICS".to_owned(),
                value: Some("true".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_METRICS_BIND_ADDRESS".to_owned(),
                value: Some("0.0.0.0:9465".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_SWARM_ADDRESSES".to_owned(),
                value: Some("/ip4/0.0.0.0/tcp/4001".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_STORE_DIR".to_owned(),
                value: Some("/data/ipfs".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_NETWORK".to_owned(),
                value: Some("local".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_LOCAL_NETWORK_ID".to_owned(),
                // We can use a hard coded value since nodes from other networks should not be
                // able to connect.
                value: Some("0".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_KADEMLIA_REPLICATION".to_owned(),
                value: Some("6".to_owned()),
                ..Default::default()
            },
            EnvVar {
                name: "CERAMIC_ONE_KADEMLIA_PARALLELISM".to_owned(),
                value: Some("1".to_owned()),
                ..Default::default()
            },
        ];
        if let Some(extra_env) = &self.env {
            extra_env.iter().for_each(|(key, value)| {
                if let Some((pos, _)) = env.iter().enumerate().find(|(_, var)| &var.name == key) {
                    env.swap_remove(pos);
                }
                env.push(EnvVar {
                    name: key.to_string(),
                    value: Some(value.to_string()),
                    ..Default::default()
                })
            });
        }
        // Sort env vars so we can have stable tests
        env.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Container {
            env: Some(env),
            image: Some(self.image.to_owned()),
            image_pull_policy: Some(self.image_pull_policy.to_owned()),
            name: IPFS_CONTAINER_NAME.to_owned(),
            ports: Some(vec![
                ContainerPort {
                    container_port: 4001,
                    name: Some("swarm-tcp".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
                ContainerPort {
                    container_port: CERAMIC_SERVICE_IPFS_PORT,
                    name: Some("rpc".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
                ContainerPort {
                    container_port: 9465,
                    name: Some("metrics".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
            ]),
            resources: Some(ResourceRequirements {
                limits: Some(self.resource_limits.clone().into()),
                requests: Some(self.resource_limits.clone().into()),
                ..Default::default()
            }),
            volume_mounts: Some(vec![VolumeMount {
                mount_path: "/data/ipfs".to_owned(),
                name: IPFS_DATA_PV_CLAIM.to_owned(),
                ..Default::default()
            }]),
            ..Default::default()
        }
    }
}

impl GoIpfsConfig {
    fn config_maps(&self, info: &CeramicInfo) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut ipfs_config = vec![(
            "001-config.sh".to_owned(),
            r#"#!/bin/sh
set -ex
# Do not bootstrap against public nodes
ipfs bootstrap rm all
# Do not sticky peer with ceramic specific peers
# We want an isolated network
ipfs config --json Peering.Peers '[]'
# Disable the gateway
ipfs config  --json Addresses.Gateway '[]'
# Enable pubsub
ipfs config  --json PubSub.Enabled true
# Only listen on specific tcp address as nothing else is exposed
ipfs config  --json Addresses.Swarm '["/ip4/0.0.0.0/tcp/4001"]'
# Set explicit resource manager limits as Kubo computes them based off
# the k8s node resources and not the pods limits.
ipfs config Swarm.ResourceMgr.MaxMemory '400 MB'
ipfs config --json Swarm.ResourceMgr.MaxFileDescriptors 500000
"#
            .to_owned(),
        )];
        if !self.commands.is_empty() {
            ipfs_config.push((
                "002-config.sh".to_owned(),
                [
                    vec!["#!/bin/sh", "set -ex"],
                    self.commands.iter().map(AsRef::as_ref).collect(),
                ]
                .concat()
                .join("\n"),
            ));
        }
        BTreeMap::from_iter(vec![(
            info.new_name("ipfs-container-init"),
            BTreeMap::from_iter(ipfs_config),
        )])
    }
    fn container(&self, info: &CeramicInfo) -> Container {
        let mut volume_mounts = vec![
            VolumeMount {
                mount_path: "/data/ipfs".to_owned(),
                name: IPFS_DATA_PV_CLAIM.to_owned(),
                ..Default::default()
            },
            VolumeMount {
                mount_path: "/container-init.d/001-config.sh".to_owned(),
                name: info.new_name("ipfs-container-init"),
                // Use an explict subpath otherwise, k8s uses symlinks which breaks
                // kubo's init logic.
                sub_path: Some("001-config.sh".to_owned()),
                ..Default::default()
            },
        ];
        if !self.commands.is_empty() {
            volume_mounts.push(VolumeMount {
                mount_path: "/container-init.d/002-config.sh".to_owned(),
                name: info.new_name("ipfs-container-init"),
                sub_path: Some("002-config.sh".to_owned()),
                ..Default::default()
            })
        }
        Container {
            image: Some(self.image.to_owned()),
            image_pull_policy: Some(self.image_pull_policy.to_owned()),
            name: IPFS_CONTAINER_NAME.to_owned(),
            ports: Some(vec![
                ContainerPort {
                    container_port: 4001,
                    name: Some("swarm-tcp".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
                ContainerPort {
                    container_port: CERAMIC_SERVICE_IPFS_PORT,
                    name: Some("rpc".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
                ContainerPort {
                    container_port: 9465,
                    name: Some("metrics".to_owned()),
                    protocol: Some("TCP".to_owned()),
                    ..Default::default()
                },
            ]),
            resources: Some(ResourceRequirements {
                limits: Some(self.resource_limits.clone().into()),
                requests: Some(self.resource_limits.clone().into()),
                ..Default::default()
            }),
            volume_mounts: Some(volume_mounts),
            ..Default::default()
        }
    }
    fn volumes(&self, info: &CeramicInfo) -> Vec<Volume> {
        vec![Volume {
            name: info.new_name("ipfs-container-init"),
            config_map: Some(ConfigMapVolumeSource {
                default_mode: Some(0o755),
                name: Some(info.new_name("ipfs-container-init")),
                ..Default::default()
            }),
            ..Default::default()
        }]
    }
}

pub fn stateful_set_spec(ns: &str, bundle: &CeramicBundle<'_>) -> StatefulSetSpec {
    let mut db_connection_string: String = "sqlite:///ceramic-data/ceramic.db".to_owned();
    if bundle.config.db_type.eq(DB_TYPE_POSTGRES) {
        db_connection_string = format!("postgres://{}:{}@{}:5432/{}", bundle.config.postgres.user_name.clone().unwrap(),bundle.config.postgres.password.clone().unwrap(),CERAMIC_POSTGRES_SERVICE_NAME.to_owned(), bundle.config.postgres.db_name.clone().unwrap())
    }

    let mut ceramic_env = vec![
        EnvVar {
            name: "CERAMIC_NETWORK".to_owned(),
            value: Some(bundle.net_config.network_type.to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "CERAMIC_NETWORK_TOPIC".to_owned(),
            value: Some(bundle.net_config.pubsub_topic.to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "ETH_RPC_URL".to_owned(),
            value: Some(bundle.net_config.eth_rpc_url.to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "CAS_API_URL".to_owned(),
            value: Some(bundle.net_config.cas_api_url.to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "CERAMIC_STATE_STORE_PATH".to_owned(),
            value: Some("/ceramic-data/statestore".to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "CERAMIC_IPFS_HOST".to_owned(),
            value: Some(format!("http://localhost:{CERAMIC_SERVICE_IPFS_PORT}")),
            ..Default::default()
        },
        EnvVar {
            name: "CERAMIC_CORS_ALLOWED_ORIGINS".to_owned(),
            value: Some(".*".to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "CERAMIC_LOG_LEVEL".to_owned(),
            value: Some("2".to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "DB_CONNECTION_STRING".to_owned(),
            value: Some(db_connection_string),
            ..Default::default()
        },
        EnvVar {
            name: "ENABLE_HISTORICAL_SYNC".to_owned(),
            value: Some(bundle.config.enable_historical_sync.to_string()),
            ..Default::default()
        },
    ];

    let mut init_env = vec![EnvVar {
        name: "CERAMIC_ADMIN_PRIVATE_KEY".to_owned(),
        value_from: Some(EnvVarSource {
            secret_key_ref: Some(SecretKeySelector {
                key: "private-key".to_owned(),
                name: Some("ceramic-admin".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }];
    init_env.append(&mut ceramic_env.clone());

    bundle.datadog.inject_env(&mut ceramic_env);

    let mut volumes = vec![
        Volume {
            empty_dir: Some(EmptyDirVolumeSource::default()),
            name: "config-volume".to_owned(),
            ..Default::default()
        },
        Volume {
            config_map: Some(ConfigMapVolumeSource {
                default_mode: Some(0o755),
                name: Some(bundle.config.init_config_map.to_owned()),
                ..Default::default()
            }),
            name: "ceramic-init".to_owned(),
            ..Default::default()
        },
        Volume {
            name: "ceramic-data".to_owned(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "ceramic-data".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        },
        Volume {
            name: IPFS_DATA_PV_CLAIM.to_owned(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: IPFS_DATA_PV_CLAIM.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        },
    ];

    volumes.append(&mut bundle.config.ipfs.volumes(&bundle.info));

    StatefulSetSpec {
        pod_management_policy: Some("Parallel".to_owned()),
        replicas: Some(bundle.info.replicas),
        selector: LabelSelector {
            match_labels: selector_labels(CERAMIC_APP),
            ..Default::default()
        },
        service_name: bundle.info.service.clone(),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                annotations: Some(BTreeMap::from_iter(vec![(
                    "prometheus/path".to_owned(),
                    "/metrics".to_owned(),
                )]))
                .map(|mut annotations| {
                    bundle.datadog.inject_annotations(&mut annotations);
                    annotations
                }),

                labels: selector_labels(CERAMIC_APP).map(|mut lbls| {
                    lbls.append(&mut managed_labels().unwrap());
                    bundle
                        .datadog
                        .inject_labels(&mut lbls, ns.to_owned(), "ceramic".to_owned());
                    lbls
                }),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        command: Some(vec![
                            "/js-ceramic/packages/cli/bin/ceramic.js".to_owned(),
                            "daemon".to_owned(),
                            "--config".to_owned(),
                            "/config/daemon-config.json".to_owned(),
                        ]),
                        env: Some(ceramic_env),
                        image: Some(bundle.config.image.clone()),
                        image_pull_policy: Some(bundle.config.image_pull_policy.clone()),
                        name: "ceramic".to_owned(),
                        ports: Some(vec![
                            ContainerPort {
                                container_port: CERAMIC_SERVICE_API_PORT,
                                name: Some("api".to_owned()),
                                ..Default::default()
                            },
                            ContainerPort {
                                container_port: 9464,
                                name: Some("metrics".to_owned()),
                                protocol: Some("TCP".to_owned()),
                                ..Default::default()
                            },
                        ]),
                        readiness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/api/v0/node/healthcheck".to_owned()),
                                port: IntOrString::String("api".to_owned()),
                                ..Default::default()
                            }),
                            initial_delay_seconds: Some(10),
                            period_seconds: Some(1),
                            timeout_seconds: Some(30),
                            ..Default::default()
                        }),
                        liveness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/api/v0/node/healthcheck".to_owned()),
                                port: IntOrString::String("api".to_owned()),
                                ..Default::default()
                            }),
                            initial_delay_seconds: Some(20),
                            period_seconds: Some(3),
                            timeout_seconds: Some(30),
                            ..Default::default()
                        }),

                        resources: Some(ResourceRequirements {
                            limits: Some(bundle.config.resource_limits.clone().into()),
                            requests: Some(bundle.config.resource_limits.clone().into()),
                            ..Default::default()
                        }),
                        volume_mounts: Some(vec![
                            VolumeMount {
                                mount_path: "/config".to_owned(),
                                name: "config-volume".to_owned(),
                                ..Default::default()
                            },
                            VolumeMount {
                                mount_path: "/ceramic-data".to_owned(),
                                name: "ceramic-data".to_owned(),
                                ..Default::default()
                            },
                        ]),
                        ..Default::default()
                    },
                    bundle.config.ipfs.container(&bundle.info),
                ],
                init_containers: Some(vec![Container {
                    command: Some(vec![
                        "/bin/bash".to_owned(),
                        "-c".to_owned(),
                        "/ceramic-init/ceramic-init.sh".to_owned(),
                    ]),
                    env: Some(init_env),
                    image: Some(bundle.config.image.to_owned()),
                    image_pull_policy: Some(bundle.config.image_pull_policy.to_owned()),
                    name: "init-ceramic-config".to_owned(),
                    resources: Some(ResourceRequirements {
                        limits: Some(bundle.config.resource_limits.clone().into()),
                        requests: Some(bundle.config.resource_limits.clone().into()),
                        ..Default::default()
                    }),
                    volume_mounts: Some(vec![
                        VolumeMount {
                            mount_path: "/config".to_owned(),
                            name: "config-volume".to_owned(),
                            ..Default::default()
                        },
                        VolumeMount {
                            mount_path: "/ceramic-init".to_owned(),
                            name: "ceramic-init".to_owned(),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }]),
                volumes: Some(volumes),
                ..Default::default()
            }),
        },
        update_strategy: Some(StatefulSetUpdateStrategy {
            rolling_update: Some(RollingUpdateStatefulSetStrategy {
                max_unavailable: Some(IntOrString::String("50%".to_owned())),
                ..Default::default()
            }),
            ..Default::default()
        }),
        volume_claim_templates: Some(vec![
            PersistentVolumeClaim {
                metadata: ObjectMeta {
                    name: Some("ceramic-data".to_owned()),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeClaimSpec {
                    access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                    resources: Some(ResourceRequirements {
                        requests: Some(BTreeMap::from_iter(vec![(
                            "storage".to_owned(),
                            Quantity("10Gi".to_owned()),
                        )])),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            PersistentVolumeClaim {
                metadata: ObjectMeta {
                    name: Some(IPFS_DATA_PV_CLAIM.to_owned()),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeClaimSpec {
                    access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                    resources: Some(ResourceRequirements {
                        requests: Some(BTreeMap::from_iter(vec![(
                            "storage".to_owned(),
                            Quantity("10Gi".to_owned()),
                        )])),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

pub fn postgres_stateful_set_spec(bundle: &CeramicBundle<'_>) -> StatefulSetSpec {
    let postgres_config = &bundle.config.postgres;
    StatefulSetSpec {
        replicas: Some(1),
        selector: LabelSelector {
            match_labels: selector_labels(CERAMIC_POSTGRES_APP),
            ..Default::default()
        },
        service_name: CERAMIC_POSTGRES_SERVICE_NAME.to_owned(),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: selector_labels(CERAMIC_POSTGRES_APP),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                containers: vec![Container {
                    env: Some(vec![
                        EnvVar {
                            name: "POSTGRES_DB".to_owned(),
                            value: postgres_config.db_name.clone(),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "POSTGRES_PASSWORD".to_owned(),
                            value: postgres_config.password.clone(),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "POSTGRES_USER".to_owned(),
                            value: postgres_config.user_name.clone(),
                            ..Default::default()
                        },
                    ]),
                    image: Some("postgres:15-alpine".to_owned()),
                    image_pull_policy: Some("IfNotPresent".to_owned()),
                    name: "postgres".to_owned(),
                    ports: Some(vec![ContainerPort {
                        container_port: 5432,
                        name: Some("postgres".to_owned()),
                        ..Default::default()
                    }]),
                    resources: Some(ResourceRequirements {
                        limits: Some(
                            (ResourceLimitsConfig {
                                cpu: Quantity("1".to_owned()),
                                memory: Quantity("1Gi".to_owned()),
                                storage: Quantity("2Gi".to_owned()),
                            })
                            .into(),
                        ),
                        requests: Some(
                            (ResourceLimitsConfig {
                                cpu: Quantity("1".to_owned()),
                                memory: Quantity("512Mi".to_owned()),
                                storage: Quantity("2Gi".to_owned()),
                            })
                            .into(),
                        ),
                        ..Default::default()
                    }),
                    volume_mounts: Some(vec![VolumeMount {
                        mount_path: "/var/lib/postgresql".to_owned(),
                        name: "postgres-data".to_owned(),
                        sub_path: Some("ceramic_data".to_owned()),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                security_context: Some(PodSecurityContext {
                    fs_group: Some(70),
                    run_as_group: Some(70),
                    run_as_user: Some(70),
                    ..Default::default()
                }),
                volumes: Some(vec![Volume {
                    name: "postgres-data".to_owned(),
                    persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                        claim_name: "postgres-data".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        },
        volume_claim_templates: Some(vec![PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("postgres-data".to_owned()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                resources: Some(ResourceRequirements {
                    requests: Some(BTreeMap::from_iter(vec![(
                        "storage".to_owned(),
                        Quantity("10Gi".to_owned()),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

pub fn postgres_service_spec() -> ServiceSpec {
    ServiceSpec {
        ports: Some(vec![ServicePort {
            name: Some("postgres".to_owned()),
            port: 5432,
            target_port: Some(IntOrString::Int(5432)),
            ..Default::default()
        }]),
        selector: selector_labels(CERAMIC_POSTGRES_APP),
        type_: Some("ClusterIP".to_owned()),
        ..Default::default()
    }
}
