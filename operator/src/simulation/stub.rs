//! Helper methods only available for tests

use std::collections::BTreeMap;

use anyhow::Result;
use expect_patch::{ExpectPatch, Expectation};
use expect_test::{expect_file, ExpectFile};
use hyper::Body;
use k8s_openapi::api::{
    apps::v1::{StatefulSet, StatefulSetStatus},
    batch::v1::{Job, JobStatus},
    core::v1::ConfigMap,
};
use keramik_common::peer_info::PeerInfo;
use kube::Resource;
use serde::Serialize;
use tracing::debug;

use crate::simulation::{Simulation, SimulationSpec, SimulationStatus};
use crate::utils::test::Request;

// We wrap tower_test::mock::Handle
type ApiServerHandle = tower_test::mock::Handle<http::Request<Body>, http::Response<Body>>;
pub struct ApiServerVerifier(ApiServerHandle);

// Add tests specific implementation to the Network
impl Simulation {
    /// A normal test network
    pub fn test() -> Self {
        let mut sim = Simulation::new("test", SimulationSpec::default());
        let meta = sim.meta_mut();
        meta.namespace = Some("test".to_owned());
        sim.with_status(SimulationStatus { nonce: 42 })
    }
    /// Modify a network to have an expected spec
    pub fn with_spec(self, spec: SimulationSpec) -> Self {
        Self { spec, ..self }
    }
    /// Modify a network to have an expected status
    pub fn with_status(self, status: SimulationStatus) -> Self {
        Self {
            status: Some(status),
            ..self
        }
    }
}

/// Stub of expected requests during reconciliation.
///
/// TODO update example
/// ```no_run
/// let mut stub = Stub::default();
/// // Patch the cas_service expected value.
/// // This patches the expected request the controller will make from its default.
/// // Default expecations are found in `./testdata/default_stubs`.
/// // Use `UPDATE_EXPECT=1 cargo test` to update all expect! including this patch.
/// stub.cas_service.patch(expect![[r#"..."#]]);
/// ```
#[derive(Debug)]
pub struct Stub {
    simulation: Simulation,
    pub peers_config_map: (ExpectPatch<ExpectFile>, ConfigMap),
    pub jaeger_service: ExpectPatch<ExpectFile>,
    pub jaeger_stateful_set: ExpectPatch<ExpectFile>,
    pub prom_config: ExpectPatch<ExpectFile>,
    pub prom_stateful_set: ExpectPatch<ExpectFile>,
    pub monitoring_service_account: ExpectPatch<ExpectFile>,
    pub monitoring_cluster_role: ExpectPatch<ExpectFile>,
    pub monitoring_cluster_role_binding: ExpectPatch<ExpectFile>,
    pub otel_config: ExpectPatch<ExpectFile>,
    pub otel_service: ExpectPatch<ExpectFile>,
    pub otel_stateful_set: ExpectPatch<ExpectFile>,

    pub jaeger_status: (ExpectPatch<ExpectFile>, StatefulSet),
    pub prom_status: (ExpectPatch<ExpectFile>, StatefulSet),
    pub otel_status: (ExpectPatch<ExpectFile>, StatefulSet),

    pub goose_service: ExpectPatch<ExpectFile>,
    pub manager_job: ExpectPatch<ExpectFile>,

    pub manager_status: (ExpectPatch<ExpectFile>, Job),

    pub worker_jobs: Vec<ExpectPatch<ExpectFile>>,

    pub status: ExpectPatch<ExpectFile>,
}

impl Stub {
    pub fn with_simulation(self, simulation: Simulation) -> Self {
        Self { simulation, ..self }
    }
}

// Implement default stub that defines two peers and all statuses are immediately ready.
impl Default for Stub {
    fn default() -> Self {
        Self {
            simulation: Simulation::test(),
            peers_config_map: (
                expect_file!["./testdata/default_stubs/peers_config_map"].into(),
                {
                    let peers = vec![
                        PeerInfo {
                            index: 0,
                            peer_id: "0".to_owned(),
                            ipfs_rpc_addr: "ipfs_rpc_addr_0".to_owned(),
                            ceramic_addr: "ceramic_addr_0".to_owned(),
                            p2p_addrs: vec!["p2p_addr_0".to_owned(), "p2p_addr_1".to_owned()],
                        },
                        PeerInfo {
                            index: 1,
                            peer_id: "1".to_owned(),
                            ipfs_rpc_addr: "ipfs_rpc_addr_1".to_owned(),
                            ceramic_addr: "ceramic_addr_1".to_owned(),
                            p2p_addrs: vec!["p2p_addr_0".to_owned(), "p2p_addr_1".to_owned()],
                        },
                    ];

                    let json_bytes = serde_json::to_string(&peers)
                        .expect("should be able to serialize PeerInfo");
                    ConfigMap {
                        data: Some(BTreeMap::from_iter([("peers.json".to_owned(), json_bytes)])),
                        ..Default::default()
                    }
                },
            ),
            jaeger_service: expect_file!["./testdata/default_stubs/jaeger_service"].into(),
            jaeger_stateful_set: expect_file!["./testdata/default_stubs/jaeger_stateful_set"]
                .into(),
            prom_config: expect_file!["./testdata/default_stubs/prom_config"].into(),
            prom_stateful_set: expect_file!["./testdata/default_stubs/prom_stateful_set"].into(),
            monitoring_service_account: expect_file![
                "./testdata/default_stubs/monitoring_service_account"
            ]
            .into(),
            monitoring_cluster_role: expect_file![
                "./testdata/default_stubs/monitoring_cluster_role"
            ]
            .into(),
            monitoring_cluster_role_binding: expect_file![
                "./testdata/default_stubs/monitoring_cluster_role_binding"
            ]
            .into(),
            otel_config: expect_file!["./testdata/default_stubs/otel_config"].into(),
            otel_service: expect_file!["./testdata/default_stubs/otel_service"].into(),
            otel_stateful_set: expect_file!["./testdata/default_stubs/otel_stateful_set"].into(),
            jaeger_status: (
                expect_file!["./testdata/default_stubs/jaeger_status"].into(),
                StatefulSet {
                    status: Some(StatefulSetStatus {
                        ready_replicas: Some(1),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            prom_status: (
                expect_file!["./testdata/default_stubs/prom_status"].into(),
                StatefulSet {
                    status: Some(StatefulSetStatus {
                        ready_replicas: Some(1),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            otel_status: (
                expect_file!["./testdata/default_stubs/otel_status"].into(),
                StatefulSet {
                    status: Some(StatefulSetStatus {
                        ready_replicas: Some(1),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            goose_service: expect_file!["./testdata/default_stubs/goose_service"].into(),
            manager_job: expect_file!["./testdata/default_stubs/manager_job"].into(),
            manager_status: (
                expect_file!["./testdata/default_stubs/manager_status"].into(),
                Job {
                    status: Some(JobStatus {
                        ready: Some(1),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            worker_jobs: vec![
                expect_file!["./testdata/default_stubs/worker_job_0"].into(),
                expect_file!["./testdata/default_stubs/worker_job_1"].into(),
            ],
            status: expect_file!["./testdata/default_stubs/status"].into(),
        }
    }
}

impl ApiServerVerifier {
    /// Create an ApiServerVerifier from a handle
    pub fn new(handle: ApiServerHandle) -> Self {
        Self(handle)
    }
    /// Run a test with the given stub.
    ///
    /// NB: If the controller is making more calls than we are handling in the stub,
    /// you then typically see a `KubeError(Service(Closed(())))` from the reconciler.
    ///
    /// You should await the `JoinHandle` (with a timeout) from this function to ensure that the
    /// stub runs to completion (i.e. all expected calls were responded to),
    /// using the timeout to catch missing api calls to Kubernetes.
    pub fn run(mut self, stub: Stub) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // We need to handle each expected call in sequence

            // First we handle the call to get the peers config map.
            self.handle_request_response(stub.peers_config_map.0, &stub.peers_config_map.1)
                .await
                .expect("peers_config_map should be reported");

            // Next we handle a sequence of apply calls
            self.handle_apply(stub.jaeger_service)
                .await
                .expect("jaeger service should apply");
            self.handle_apply(stub.jaeger_stateful_set)
                .await
                .expect("jaeger stateful set should apply");
            self.handle_apply(stub.prom_config)
                .await
                .expect("prom-config configmap should apply");
            self.handle_apply(stub.prom_stateful_set)
                .await
                .expect("prom stateful set should apply");
            self.handle_apply(stub.monitoring_service_account)
                .await
                .expect("monitoring service account should apply");
            self.handle_apply(stub.monitoring_cluster_role)
                .await
                .expect("monitoring cluster role should apply");
            self.handle_apply(stub.monitoring_cluster_role_binding)
                .await
                .expect("monitoring cluster role binding should apply");
            self.handle_apply(stub.otel_config)
                .await
                .expect("otel config map should apply");
            self.handle_apply(stub.otel_service)
                .await
                .expect("otel service should apply");
            self.handle_apply(stub.otel_stateful_set)
                .await
                .expect("otel stateful set should apply");

            // Next we handle a sequence of status calls for various services
            self.handle_request_response(stub.jaeger_status.0, &stub.jaeger_status.1)
                .await
                .expect("should report jaeger status");
            self.handle_request_response(stub.prom_status.0, &stub.prom_status.1)
                .await
                .expect("should report jaeger status");
            self.handle_request_response(stub.otel_status.0, &stub.otel_status.1)
                .await
                .expect("should report jaeger status");

            // Next we handle creating the jobs
            self.handle_apply(stub.goose_service)
                .await
                .expect("goose service should apply");
            self.handle_apply(stub.manager_job)
                .await
                .expect("manager job should apply");

            self.handle_request_response(stub.manager_status.0, &stub.manager_status.1)
                .await
                .expect("manager should report status");

            for w in stub.worker_jobs {
                self.handle_apply(w).await.expect("should be next request");
            }

            // Finally we handle the patch status call
            self.handle_patch_status(stub.status, stub.simulation.clone())
                .await
                .expect("status should patch");
        })
    }

    async fn handle_patch_status(
        &mut self,
        expected_request: impl Expectation,
        simulation: Simulation,
    ) -> Result<()> {
        let (request, send) = self.0.next_request().await.expect("service not called");
        let request = Request::from_request(request).await?;
        expected_request.assert_debug_eq(&request);

        let json: serde_json::Value =
            serde_json::from_str(&request.body.0).expect("status should be JSON");

        let status_json = json.get("status").expect("status object").clone();
        let status: SimulationStatus =
            serde_json::from_value(status_json).expect("JSON should be a valid status");

        let simulation = simulation.with_status(status);
        let response = serde_json::to_vec(&simulation).unwrap();
        send.send_response(
            http::Response::builder()
                .body(Body::from(response))
                .unwrap(),
        );
        Ok(())
    }

    async fn handle_apply(&mut self, expected_request: impl Expectation) -> Result<()> {
        let (request, send) = self.0.next_request().await.expect("service not called");
        let request = Request::from_request(request).await?;
        expected_request.assert_debug_eq(&request);

        send.send_response(
            http::Response::builder()
                .body(Body::from(request.body.0))
                .unwrap(),
        );
        Ok(())
    }
    async fn handle_request_response<T>(
        &mut self,
        expected_request: impl Expectation,
        response: &T,
    ) -> Result<()>
    where
        T: ?Sized + Serialize + std::fmt::Debug,
    {
        let (request, send) = self.0.next_request().await.expect("service not called");
        let request = Request::from_request(request).await?;
        expected_request.assert_debug_eq(&request);

        debug!(?response, "sending response");
        let response = serde_json::to_vec(response).unwrap();
        send.send_response(
            http::Response::builder()
                .body(Body::from(response))
                .unwrap(),
        );
        Ok(())
    }
}