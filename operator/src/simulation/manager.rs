use std::collections::BTreeMap;

use k8s_openapi::api::{
    batch::v1::JobSpec,
    core::v1::{
        ConfigMapVolumeSource, Container, EnvVar, PodSpec, PodTemplateSpec, ServicePort,
        ServiceSpec, Volume, VolumeMount,
    },
};
use kube::core::ObjectMeta;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::network::controller::PEERS_CONFIG_MAP_NAME;

use rand::random;

pub fn service_spec() -> ServiceSpec {
    ServiceSpec {
        ports: Some(vec![ServicePort {
            port: 5115,
            name: Some("manager".to_owned()),
            ..Default::default()
        }]),
        selector: Some(BTreeMap::from_iter(vec![(
            "name".to_owned(),
            "goose".to_owned(),
        )])),
        cluster_ip: Some("None".to_owned()),
        ..Default::default()
    }
}

/// ManagerSpec defines a goose manager
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSpec {
    pub scenario: Option<String>,
    pub users: Option<u32>,
    pub run_time: Option<u32>,
    pub nonce: Option<u32>,
}

// ManagerConfig defines which properties of the JobSpec can be customized.
pub struct ManagerConfig {
    pub scenario: String,
    pub users: u32,
    pub run_time: u32,
    pub nonce: u32,
}

// Define clear defaults for this config
impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            scenario: "ceramic-simple".to_owned(),
            users: 100,
            run_time: 10,
            nonce: random::<u32>(),
        }
    }
}

impl From<Option<ManagerSpec>> for ManagerConfig {
    fn from(value: Option<ManagerSpec>) -> Self {
        match value {
            Some(spec) => spec.into(),
            None => ManagerConfig::default(),
        }
    }
}

impl From<ManagerSpec> for ManagerConfig {
    fn from(value: ManagerSpec) -> Self {
        let default = Self::default();
        Self {
            scenario: value.scenario.unwrap_or(default.scenario),
            users: value.users.unwrap_or(default.users),
            run_time: value.run_time.unwrap_or(default.run_time),
            nonce: value.nonce.unwrap_or(default.nonce),
        }
    }
}

pub fn manager_job_spec(config: impl Into<ManagerConfig>) -> JobSpec {
    let config = config.into();
    JobSpec {
        backoff_limit: Some(4),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(BTreeMap::from_iter(vec![(
                    "name".to_owned(),
                    "goose".to_owned(),
                )])),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                hostname: Some("manager".to_owned()),
                subdomain: Some("goose".to_owned()),
                containers: vec![Container {
                    name: "manager".to_owned(),
                    image: Some("keramik/runner:dev".to_owned()),
                    image_pull_policy: Some("IfNotPresent".to_owned()),
                    command: Some(vec![
                        "/usr/bin/keramik-runner".to_owned(),
                        "simulate".to_owned(),
                    ]),
                    env: Some(vec![
                        EnvVar {
                            name: "RUNNER_OTLP_ENDPOINT".to_owned(),
                            value: Some("http://otel:4317".to_owned()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "RUST_LOG".to_owned(),
                            value: Some("info,keramik_runner=trace".to_owned()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_SCENARIO".to_owned(),
                            value: Some(config.scenario.to_owned()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_MANAGER".to_owned(),
                            value: Some("true".to_owned()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_PEERS_PATH".to_owned(),
                            value: Some("/keramik-peers/peers.json".to_owned()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_TARGET_PEER".to_owned(),
                            value: Some(0.to_string()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_NONCE".to_owned(),
                            value: Some(config.nonce.to_string()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_USERS".to_owned(),
                            value: Some(config.users.to_string()),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "SIMULATE_RUN_TIME".to_owned(),
                            value: Some(format!("{}m", config.run_time)),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "DID_KEY".to_owned(),
                            value: Some(
                                "did:key:z6Mkqn5jbycThHcBtakJZ8fHBQ2oVRQhXQEdQk5ZK2NDtNZA"
                                    .to_owned(),
                            ),
                            ..Default::default()
                        },
                        EnvVar {
                            name: "DID_PRIVATE_KEY".to_owned(),
                            value: Some(
                                "86dce513cf0a37d4acd6d2c2e00fe4b95e0e655ca51e1a890808f5fa6f4fe65a"
                                    .to_owned(),
                            ),
                            ..Default::default()
                        },
                    ]),
                    volume_mounts: Some(vec![VolumeMount {
                        mount_path: "/keramik-peers".to_owned(),
                        name: "keramik-peers".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                volumes: Some(vec![Volume {
                    config_map: Some(ConfigMapVolumeSource {
                        default_mode: Some(0o755),
                        name: Some(PEERS_CONFIG_MAP_NAME.to_owned()),
                        ..Default::default()
                    }),
                    name: "keramik-peers".to_owned(),
                    ..Default::default()
                }]),
                restart_policy: Some("Never".to_owned()),
                ..Default::default()
            }),
        },
        ..Default::default()
    }
}