use std::{sync::Arc, time::Duration};

use futures::stream::StreamExt;
use k8s_openapi::api::{
    apps::v1::StatefulSet,
    batch::v1::Job,
    core::v1::{ConfigMap, Namespace, Pod, Service},
};

use kube::{
    api::{Patch, PatchParams},
    client::Client,
    core::object::HasSpec,
    runtime::Controller,
    Api,
};
use kube::{
    runtime::{
        controller::Action,
        watcher::{self, Config},
    },
    Resource, ResourceExt,
};
use rand::{thread_rng, Rng, RngCore};

use tracing::{debug, error};

use crate::{
    labels::MANAGED_BY_LABEL_SELECTOR,
    simulation::{
        job::JobImageConfig, manager, manager::ManagerConfig, redis, worker, worker::WorkerConfig,
        Simulation, SimulationStatus,
    },
    utils::Clock,
};

use crate::monitoring::{jaeger, opentelemetry, prometheus};

use crate::network::{
    ipfs_rpc::{HttpRpcClient, IpfsRpcClient},
    peers::PEERS_MAP_KEY,
    Network, PEERS_CONFIG_MAP_NAME,
};

use keramik_common::peer_info::Peer;

use crate::utils::{
    apply_account, apply_cluster_role, apply_cluster_role_binding, apply_config_map, apply_job,
    apply_service, apply_stateful_set, Context,
};

/// Handle errors during reconciliation.
fn on_error(
    _network: Arc<Simulation>,
    _error: &Error,
    _context: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
) -> Action {
    Action::requeue(Duration::from_secs(5))
}

/// Errors produced by the reconcile function.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("App error: {source}")]
    App {
        #[from]
        source: anyhow::Error,
    },
    #[error("Kube error: {source}")]
    Kube {
        #[from]
        source: kube::Error,
    },
}

/// Start a controller for the Simulation CRD.
pub async fn run() {
    let k_client = Client::try_default().await.unwrap();
    let context = Arc::new(
        Context::new(k_client.clone(), HttpRpcClient).expect("should be able to create context"),
    );

    // Add api for other resources, ie ceramic nodes
    let networks: Api<Network> = Api::all(k_client.clone());
    let simulations: Api<Simulation> = Api::all(k_client.clone());
    let namespaces: Api<Namespace> = Api::all(k_client.clone());
    let services = Api::<Service>::all(k_client.clone());
    let config_maps = Api::<ConfigMap>::all(k_client.clone());
    let jobs = Api::<Job>::all(k_client.clone());
    let pods = Api::<Pod>::all(k_client.clone());

    Controller::new(simulations.clone(), Config::default())
        .owns(
            networks,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .owns(
            namespaces,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .owns(
            services,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .owns(
            config_maps,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .owns(
            jobs,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .owns(
            pods,
            watcher::Config::default().labels(MANAGED_BY_LABEL_SELECTOR),
        )
        .run(reconcile, on_error, context)
        .for_each(|rec_res| async move {
            match rec_res {
                Ok((simulation, _)) => {
                    debug!(simulation.name, "reconcile success");
                }
                Err(err) => {
                    error!(?err, "reconcile error")
                }
            }
        })
        .await;
}

/// Perform a reconile pass for the Simulation CRD
async fn reconcile(
    simulation: Arc<Simulation>,
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
) -> Result<Action, Error> {
    let spec = simulation.spec();
    debug!(?spec, "reconcile");

    let status = if let Some(status) = &simulation.status {
        status.clone()
    } else {
        // Generate new status with random nonce
        SimulationStatus {
            nonce: thread_rng().gen(),
        }
    };

    let ns = simulation.namespace().unwrap();
    let num_peers = get_num_peers(cx.clone(), &ns).await?;

    apply_jaeger(cx.clone(), &ns, simulation.clone()).await?;
    apply_prometheus(cx.clone(), &ns, simulation.clone()).await?;
    apply_opentelemetry(cx.clone(), &ns, simulation.clone()).await?;

    let ready = monitoring_ready(cx.clone(), &ns).await?;

    if !ready {
        return Ok(Action::requeue(Duration::from_secs(10)));
    }

    apply_redis(cx.clone(), &ns, simulation.clone()).await?;
    let ready = redis_ready(cx.clone(), &ns).await?;
    if !ready {
        return Ok(Action::requeue(Duration::from_secs(10)));
    }

    let job_image_config = JobImageConfig::from(spec);

    let manager_config = ManagerConfig {
        scenario: spec.scenario.to_owned(),
        users: spec.users.to_owned(),
        run_time: spec.run_time.to_owned(),
        nonce: status.nonce,
        job_image_config: job_image_config.clone(),
        throttle_requests: spec.throttle_requests,
    };

    apply_manager(cx.clone(), &ns, simulation.clone(), manager_config).await?;

    let jobs: Api<Job> = Api::namespaced(cx.k_client.clone(), &ns);
    let manager_job = jobs.get_status(MANAGER_JOB_NAME).await?;
    let manager_ready = manager_job.status.unwrap().ready.unwrap_or_default();

    if manager_ready > 0 {
        //for loop n peers
        apply_n_workers(
            cx.clone(),
            &ns,
            num_peers,
            status.nonce,
            simulation.clone(),
            job_image_config.clone(),
        )
        .await?;
    }

    let simulations: Api<Simulation> = Api::namespaced(cx.k_client.clone(), &ns);
    let _patched = simulations
        .patch_status(
            &simulation.name_any(),
            &PatchParams::default(),
            &Patch::Merge(serde_json::json!({ "status": status })),
        )
        .await?;

    //TODO jobs done/fail cleanup, post process

    Ok(Action::requeue(Duration::from_secs(10)))
}

pub const MANAGER_SERVICE_NAME: &str = "goose";
pub const MANAGER_JOB_NAME: &str = "simulate-manager";
pub const WORKER_JOB_NAME: &str = "simulate-worker";

pub const JAEGER_SERVICE_NAME: &str = "jaeger";
pub const OTEL_SERVICE_NAME: &str = "otel";

pub const OTEL_CR_BINDING: &str = "monitoring-cluster-role-binding";
pub const OTEL_CR: &str = "monitoring-cluster-role";
pub const OTEL_ACCOUNT: &str = "monitoring-service-account";

pub const OTEL_CONFIG_MAP_NAME: &str = "otel-config";
pub const PROM_CONFIG_MAP_NAME: &str = "prom-config";

async fn apply_manager(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    simulation: Arc<Simulation>,
    config: ManagerConfig,
) -> Result<(), kube::error::Error> {
    let orefs = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    apply_service(
        cx.clone(),
        ns,
        orefs.clone(),
        MANAGER_SERVICE_NAME,
        manager::service_spec(),
    )
    .await?;
    apply_job(
        cx.clone(),
        ns,
        orefs.clone(),
        MANAGER_JOB_NAME,
        manager::manager_job_spec(config),
    )
    .await?;

    Ok(())
}

async fn get_num_peers(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
) -> Result<u32, kube::error::Error> {
    let config_maps: Api<ConfigMap> = Api::namespaced(cx.k_client.clone(), ns);
    let map = config_maps.get(PEERS_CONFIG_MAP_NAME).await?;
    let data = map.data.unwrap();
    let value = data.get(PEERS_MAP_KEY).unwrap();
    let peers: Vec<Peer> = serde_json::from_str::<Vec<Peer>>(value)
        .unwrap()
        .into_iter()
        .filter(|peer| matches!(peer, Peer::Ceramic(_)))
        .collect();

    debug!(peers = peers.len(), "get_num_peers");
    Ok(peers.len() as u32)
}

async fn redis_ready(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
) -> Result<bool, kube::error::Error> {
    let stateful_sets: Api<StatefulSet> = Api::namespaced(cx.k_client.clone(), ns);
    let redis = stateful_sets.get_status("redis").await?;

    let redis_ready = redis
        .status
        .map(|status| status.ready_replicas.unwrap_or_default() > 0)
        .unwrap_or_default();

    Ok(redis_ready)
}

async fn monitoring_ready(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
) -> Result<bool, kube::error::Error> {
    let stateful_sets: Api<StatefulSet> = Api::namespaced(cx.k_client.clone(), ns);
    let jaeger = stateful_sets.get_status("jaeger").await?;
    let prom = stateful_sets.get_status("prometheus").await?;
    let otel = stateful_sets.get_status("opentelemetry").await?;

    let jaeger_ready = jaeger
        .status
        .map(|status| status.ready_replicas.unwrap_or_default() > 0)
        .unwrap_or_default();
    let prom_ready = prom
        .status
        .map(|status| status.ready_replicas.unwrap_or_default() > 0)
        .unwrap_or_default();
    let otel_ready = otel
        .status
        .map(|status| status.ready_replicas.unwrap_or_default() > 0)
        .unwrap_or_default();

    Ok(jaeger_ready && prom_ready && otel_ready)
}

async fn apply_n_workers(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    peers: u32,
    nonce: u32,
    simulation: Arc<Simulation>,
    job_image_config: JobImageConfig,
) -> Result<(), kube::error::Error> {
    let spec = simulation.spec();
    let orefs = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    for i in 0..peers {
        let config = WorkerConfig {
            scenario: spec.scenario.to_owned(),
            target_peer: i,
            nonce,
            job_image_config: job_image_config.clone(),
        };

        apply_job(
            cx.clone(),
            ns,
            orefs.clone(),
            &(WORKER_JOB_NAME.to_owned() + "-" + &i.to_string()),
            worker::worker_job_spec(config),
        )
        .await?;
    }

    Ok(())
}

async fn apply_redis(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    simulation: Arc<Simulation>,
) -> Result<(), kube::error::Error> {
    let orefs: Vec<_> = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    apply_service(
        cx.clone(),
        ns,
        orefs.clone(),
        "redis",
        redis::service_spec(),
    )
    .await?;
    apply_stateful_set(
        cx.clone(),
        ns,
        orefs.clone(),
        "redis",
        redis::stateful_set_spec(),
    )
    .await?;

    Ok(())
}

async fn apply_jaeger(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    simulation: Arc<Simulation>,
) -> Result<(), kube::error::Error> {
    let orefs: Vec<_> = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    apply_service(
        cx.clone(),
        ns,
        orefs.clone(),
        JAEGER_SERVICE_NAME,
        jaeger::service_spec(),
    )
    .await?;

    apply_stateful_set(
        cx.clone(),
        ns,
        orefs.clone(),
        "jaeger",
        jaeger::stateful_set_spec(),
    )
    .await?;
    Ok(())
}

async fn apply_prometheus(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    simulation: Arc<Simulation>,
) -> Result<(), kube::error::Error> {
    let orefs = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    apply_config_map(
        cx.clone(),
        ns,
        orefs.clone(),
        PROM_CONFIG_MAP_NAME,
        prometheus::config_map_data(),
    )
    .await?;
    apply_stateful_set(
        cx.clone(),
        ns,
        orefs.clone(),
        "prometheus",
        prometheus::stateful_set_spec(),
    )
    .await?;
    Ok(())
}

async fn apply_opentelemetry(
    cx: Arc<Context<impl IpfsRpcClient, impl RngCore, impl Clock>>,
    ns: &str,
    simulation: Arc<Simulation>,
) -> Result<(), kube::error::Error> {
    let orefs = simulation
        .controller_owner_ref(&())
        .map(|oref| vec![oref])
        .unwrap_or_default();

    apply_account(cx.clone(), ns, orefs.clone(), OTEL_ACCOUNT).await?;
    apply_cluster_role(
        cx.clone(),
        ns,
        orefs.clone(),
        OTEL_CR,
        opentelemetry::cluster_role(),
    )
    .await?;
    apply_cluster_role_binding(
        cx.clone(),
        orefs.clone(),
        OTEL_CR_BINDING,
        opentelemetry::cluster_role_binding(ns),
    )
    .await?;
    apply_config_map(
        cx.clone(),
        ns,
        orefs.clone(),
        OTEL_CONFIG_MAP_NAME,
        opentelemetry::config_map_data(),
    )
    .await?;
    apply_service(
        cx.clone(),
        ns,
        orefs.clone(),
        OTEL_SERVICE_NAME,
        opentelemetry::service_spec(),
    )
    .await?;
    apply_stateful_set(
        cx.clone(),
        ns,
        orefs.clone(),
        "opentelemetry",
        opentelemetry::stateful_set_spec(),
    )
    .await?;

    Ok(())
}
// Stub tests relying on stub.rs and its apiserver stubs
#[cfg(test)]
mod tests {
    use super::{reconcile, Simulation};

    use crate::{
        network::ipfs_rpc::tests::MockIpfsRpcClientTest,
        simulation::{stub::Stub, SimulationSpec},
        utils::{test::ApiServerVerifier, Context},
    };

    use crate::utils::test::timeout_after_1s;

    use expect_test::{expect, expect_file};
    use k8s_openapi::api::core::v1::ConfigMap;
    use keramik_common::peer_info::{CeramicPeerInfo, Peer};
    use std::{collections::BTreeMap, sync::Arc};
    use tracing_test::traced_test;

    // This tests defines the default stubs,
    // meaning the default stubs are the request response pairs
    // that occur when reconiling a default spec and status.
    #[tokio::test]
    #[traced_test]
    async fn reconcile_from_empty() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test();
        let stub = Stub::default();
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn reconcile_scenario() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            scenario: "test-scenario".to_owned(),
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.manager_job.patch(expect![[r#"
            --- original
            +++ modified
            @@ -41,7 +41,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_MANAGER",
        "#]]);
        stub.worker_jobs[0].patch(expect![[r#"
            --- original
            +++ modified
            @@ -49,7 +49,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_TARGET_PEER",
        "#]]);
        stub.worker_jobs[1].patch(expect![[r#"
            --- original
            +++ modified
            @@ -49,7 +49,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_TARGET_PEER",
        "#]]);
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
    #[tokio::test]
    #[traced_test]
    async fn reconcile_user_count() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            users: 10,
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.manager_job.patch(expect![[r#"
            --- original
            +++ modified
            @@ -61,7 +61,7 @@
                               },
                               {
                                 "name": "SIMULATE_USERS",
            -                    "value": "0"
            +                    "value": "10"
                               },
                               {
                                 "name": "SIMULATE_RUN_TIME",
        "#]]);
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
    #[tokio::test]
    #[traced_test]
    async fn reconcile_run_time() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            run_time: 10,
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.manager_job.patch(expect![[r#"
            --- original
            +++ modified
            @@ -65,7 +65,7 @@
                               },
                               {
                                 "name": "SIMULATE_RUN_TIME",
            -                    "value": "0m"
            +                    "value": "10m"
                               },
                               {
                                 "name": "DID_KEY",
        "#]]);
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
    #[tokio::test]
    #[traced_test]
    async fn reconcile_three_peers() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.peers_config_map.1 = {
            let peers = vec![
                Peer::Ceramic(CeramicPeerInfo {
                    peer_id: "0".to_owned(),
                    ipfs_rpc_addr: "ipfs_rpc_addr_0".to_owned(),
                    ceramic_addr: "ceramic_addr_0".to_owned(),
                    p2p_addrs: vec!["p2p_addr_0".to_owned(), "p2p_addr_1".to_owned()],
                }),
                Peer::Ceramic(CeramicPeerInfo {
                    peer_id: "1".to_owned(),
                    ipfs_rpc_addr: "ipfs_rpc_addr_1".to_owned(),
                    ceramic_addr: "ceramic_addr_1".to_owned(),
                    p2p_addrs: vec!["p2p_addr_0".to_owned(), "p2p_addr_1".to_owned()],
                }),
                Peer::Ceramic(CeramicPeerInfo {
                    peer_id: "2".to_owned(),
                    ipfs_rpc_addr: "ipfs_rpc_addr_2".to_owned(),
                    ceramic_addr: "ceramic_addr_2".to_owned(),
                    p2p_addrs: vec!["p2p_addr_0".to_owned(), "p2p_addr_1".to_owned()],
                }),
            ];

            let json_bytes =
                serde_json::to_string(&peers).expect("should be able to serialize PeerInfo");
            ConfigMap {
                data: Some(BTreeMap::from_iter([("peers.json".to_owned(), json_bytes)])),
                ..Default::default()
            }
        };
        stub.worker_jobs
            .push(expect_file!["./testdata/worker_job_2"].into());

        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
    #[tokio::test]
    #[traced_test]
    async fn reconcile_scenario_custom_images() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            scenario: "test-scenario".to_owned(),
            image: Some("image:dev".to_owned()),
            image_pull_policy: Some("IfNotPresent".to_owned()),
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.manager_job.patch(expect![[r#"
            --- original
            +++ modified
            @@ -41,7 +41,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_MANAGER",
            @@ -76,8 +76,8 @@
                                 "value": "86dce513cf0a37d4acd6d2c2e00fe4b95e0e655ca51e1a890808f5fa6f4fe65a"
                               }
                             ],
            -                "image": "public.ecr.aws/r5b3e0r5/3box/keramik-runner:latest",
            -                "imagePullPolicy": "Always",
            +                "image": "image:dev",
            +                "imagePullPolicy": "IfNotPresent",
                             "name": "manager",
                             "volumeMounts": [
                               {
        "#]]);
        stub.worker_jobs[0].patch(expect![[r#"
            --- original
            +++ modified
            @@ -49,7 +49,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_TARGET_PEER",
            @@ -72,8 +72,8 @@
                                 "value": "86dce513cf0a37d4acd6d2c2e00fe4b95e0e655ca51e1a890808f5fa6f4fe65a"
                               }
                             ],
            -                "image": "public.ecr.aws/r5b3e0r5/3box/keramik-runner:latest",
            -                "imagePullPolicy": "Always",
            +                "image": "image:dev",
            +                "imagePullPolicy": "IfNotPresent",
                             "name": "worker",
                             "volumeMounts": [
                               {
        "#]]);
        stub.worker_jobs[1].patch(expect![[r#"
            --- original
            +++ modified
            @@ -49,7 +49,7 @@
                               },
                               {
                                 "name": "SIMULATE_SCENARIO",
            -                    "value": ""
            +                    "value": "test-scenario"
                               },
                               {
                                 "name": "SIMULATE_TARGET_PEER",
            @@ -72,8 +72,8 @@
                                 "value": "86dce513cf0a37d4acd6d2c2e00fe4b95e0e655ca51e1a890808f5fa6f4fe65a"
                               }
                             ],
            -                "image": "public.ecr.aws/r5b3e0r5/3box/keramik-runner:latest",
            -                "imagePullPolicy": "Always",
            +                "image": "image:dev",
            +                "imagePullPolicy": "IfNotPresent",
                             "name": "worker",
                             "volumeMounts": [
                               {
        "#]]);
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
    #[tokio::test]
    #[traced_test]
    async fn reconcile_throttle() {
        let mock_rpc_client = MockIpfsRpcClientTest::new();
        let (testctx, api_handle) = Context::test(mock_rpc_client);
        let fakeserver = ApiServerVerifier::new(api_handle);
        let simulation = Simulation::test().with_spec(SimulationSpec {
            throttle_requests: Some(100),
            ..Default::default()
        });
        let mut stub = Stub::default();
        stub.manager_job.patch(expect![[r#"
            --- original
            +++ modified
            @@ -74,6 +74,10 @@
                               {
                                 "name": "DID_PRIVATE_KEY",
                                 "value": "86dce513cf0a37d4acd6d2c2e00fe4b95e0e655ca51e1a890808f5fa6f4fe65a"
            +                  },
            +                  {
            +                    "name": "SIMULATE_THROTTLE_REQUESTS",
            +                    "value": "100"
                               }
                             ],
                             "image": "public.ecr.aws/r5b3e0r5/3box/keramik-runner:latest",
        "#]]);
        let mocksrv = stub.run(fakeserver);
        reconcile(Arc::new(simulation), testctx)
            .await
            .expect("reconciler");
        timeout_after_1s(mocksrv).await;
    }
}
