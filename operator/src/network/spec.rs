//! Place all spec types into a single module so they can be used as a lightweight dependency
use std::collections::HashMap;

use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use keramik_common::peer_info::Peer;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Primary CRD for creating and managing a Ceramic network.
#[derive(CustomResource, Serialize, Deserialize, Debug, Default, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "keramik.3box.io",
    version = "v1alpha1",
    kind = "Network",
    plural = "networks",
    status = "NetworkStatus",
    derive = "PartialEq"
)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSpec {
    /// Number of Ceramic peers
    pub replicas: i32,
    ///  Describes how new peers in the network should be bootstrapped.
    pub bootstrap: Option<BootstrapSpec>,
    /// Describes how each peer should behave.
    /// Multiple ceramic specs can be defined.
    /// Total replicas will be split across each ceramic spec according to relative weights.
    /// It is possible that if the weight is small enough compared to others that a single spec
    /// will be assigned zero replicas.
    pub ceramic: Vec<CeramicSpec>,
    /// Name of secret containing the private key used for signing anchor requests and generating
    /// the Admin DID.
    pub private_key_secret: Option<String>,
    /// Ceramic network type
    pub network_type: Option<String>,
    /// PubSub topic for Ceramic nodes to use
    pub pubsub_topic: Option<String>,
    /// Ethereum RPC URL for Ceramic nodes to use for verifying anchors
    pub eth_rpc_url: Option<String>,
    /// URL for Ceramic Anchor Service (CAS)
    pub cas_api_url: Option<String>,
    /// Describes how CAS should be deployed.
    pub cas: Option<CasSpec>,
    /// Descibes if/how datadog should be deployed.
    pub datadog: Option<DataDogSpec>,
    /// The number of seconds this network should live.
    /// If unset the network lives forever.
    pub ttl_seconds: Option<u64>,
    /// Namespce for ceramic network
    pub namespace: Option<String>,
}

/// Current status of the network.
#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    /// Number of Ceramic peers
    pub replicas: i32,
    ///  Describes how new peers in the network should be bootstrapped.
    pub ready_replicas: i32,
    /// K8s namespace this network is deployed in
    pub namespace: Option<String>,
    /// Information about each Ceramic peer
    pub peers: Vec<Peer>,
    /// Time when the network will expire and be deleted.
    /// If unset the network lives forever.
    pub expiration_time: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
}

/// BootstrapSpec defines how the network bootstrap process should proceed.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapSpec {
    /// Image of the runner for the bootstrap job.
    pub image: Option<String>,
    /// Image pull policy for the bootstrap job.
    pub image_pull_policy: Option<String>,
    /// Bootstrap method. Defaults to ring.
    pub method: Option<String>,
    /// Number of nodes to connect to each peer.
    pub n: Option<i32>,
}

/// Describes how a Ceramic peer should behave.
#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CeramicSpec {
    /// Relative weight of the spec compared to others.
    pub weight: Option<i32>,
    /// Name of a config map with a ceramic-init.sh script that runs as an initialization step.
    pub init_config_map: Option<String>,
    /// Image of the ceramic container.
    pub image: Option<String>,
    /// Pull policy for the ceramic container image.
    pub image_pull_policy: Option<String>,
    /// Configuration of the IPFS container
    pub ipfs: Option<IpfsSpec>,
    /// Resource limits for ceramic nodes, applies to both requests and limits.
    pub resource_limits: Option<ResourceLimitsSpec>,
    /// Composedb type for ceramic nodes, for example postgres or sqlite.
    pub db_type: Option<String>,
    /// Pg configs for ceramic
    pub ceramic_postgres: Option<CeramicPostgresSpec>,
     /// Enable historical sync for ceramic nodes
     pub enable_historical_sync: Option<bool>,
}

/// Describes how the PG db for ceramic node should behave.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CeramicPostgresSpec {
    /// Name of postgres db to use
    pub db_name: Option<String>,
    /// Name of postgres user to use
    pub user_name: Option<String>,
    /// Password for the postgres user
    pub password: Option<String>,
}

/// Describes how the IPFS node for a peer should behave.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum IpfsSpec {
    /// Rust IPFS specification
    Rust(RustIpfsSpec),
    /// Go IPFS specification
    Go(GoIpfsSpec),
}

/// Describes how the Rust IPFS node for a peer should behave.
#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RustIpfsSpec {
    /// Name of image to use
    pub image: Option<String>,
    /// Image pull policy for the image
    pub image_pull_policy: Option<String>,
    /// Resource limits for ipfs nodes, applies to both requests and limits.
    pub resource_limits: Option<ResourceLimitsSpec>,
    /// Value of the RUST_LOG env var.
    pub rust_log: Option<String>,
    /// Extra env values to pass to the image.
    /// CAUTION: Any env vars specified in this set will override any predefined values.
    pub env: Option<HashMap<String, String>>,
}

/// Describes how the Go IPFS node for a peer should behave.
#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoIpfsSpec {
    /// Name of image to use
    pub image: Option<String>,
    /// Image pull policy for the image
    pub image_pull_policy: Option<String>,
    /// Resource limits for ipfs nodes, applies to both requests and limits.
    pub resource_limits: Option<ResourceLimitsSpec>,
    /// List of ipfs commands to run during initialization.
    pub commands: Option<Vec<String>>,
}

/// Defines details about how CAS is deployed
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CasSpec {
    /// Image of the runner for the bootstrap job.
    pub image: Option<String>,
    /// Image pull policy for the bootstrap job.
    pub image_pull_policy: Option<String>,
    /// Resource limits for the CAS pod, applies to both requests and limits.
    pub cas_resource_limits: Option<ResourceLimitsSpec>,
    /// Resource limits for the CAS IPFS pod, applies to both requests and limits.
    pub ipfs_resource_limits: Option<ResourceLimitsSpec>,
    /// Resource limits for the Ganache pod, applies to both requests and limits.
    pub ganache_resource_limits: Option<ResourceLimitsSpec>,
    /// Resource limits for the CAS Postgres pod, applies to both requests and limits.
    pub postgres_resource_limits: Option<ResourceLimitsSpec>,
    /// Resource limits for the LocalStack pod, applies to both requests and limits.
    pub localstack_resource_limits: Option<ResourceLimitsSpec>,
}

/// Describes if and how to configure datadog telemetry
#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataDogSpec {
    /// When true datadog telemetry will be collected.
    pub enabled: Option<bool>,
    /// Version of the DataDog agent.
    pub version: Option<String>,
    /// When true profiles will be collected.
    pub profiling_enabled: Option<bool>,
}

/// Describes the resources limits and requests for a pod
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimitsSpec {
    /// Cpu resource limit
    pub cpu: Option<Quantity>,
    /// Memory resource limit
    pub memory: Option<Quantity>,
    /// Ephemeral storage resource limit
    pub storage: Option<Quantity>,
}
