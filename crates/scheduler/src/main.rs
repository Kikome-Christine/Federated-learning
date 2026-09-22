//! Regulatory-aware, resource-aware participant scheduler (paper Sec. III-D).
//!
//! Every round it (1) probes every node's profile, (2) evaluates residency
//! policy (native Rust, and/or an Open Policy Agent sidecar), (3) scores and
//! selects. Policy is therefore enforced continuously per round, not once at
//! onboarding. Probe RTT is MEASURED, so network shaping changes decisions.

use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use flcommon::rng::{derive_seed, SplitMix64};
use flcommon::select::{select as select_nodes, NodeView, Policy, Strategy, Weights};
use flproto::fl::participant_client::ParticipantClient;
use flproto::fl::scheduler_server::{Scheduler, SchedulerServer};
use flproto::fl::{Candidate, Empty, Exclusion, OutcomeReport, Profile, SelectRequest, SelectResponse};
use tokio::task::JoinSet;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Request, Response, Status};

struct NodeCfg {
    id: String,
    endpoint: String,
    channel: Channel,
}

struct Opa {
    url: String,
    http: reqwest::Client,
}

impl Opa {
    /// Fail-closed: any error is treated as "deny".
    async fn verdict(&self, p: &Profile) -> Result<(), String> {
        let body = serde_json::json!({
            "input": {
                "jurisdiction": p.jurisdiction,
                "residency_zone": p.residency_zone,
                "license_status": p.license_status,
            }
        });
        let resp = self
            .http
            .post(format!("{}/v1/data/fl/residency", self.url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("opa_unreachable:{e}"))?;
        let v: serde_json::Value = resp.json().await.map_err(|e| format!("opa_bad_response:{e}"))?;
        if v["result"]["allow"].as_bool().unwrap_or(false) {
            Ok(())
        } else {
            let reason = v["result"]["reasons"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("opa_denied")
                .to_string();
            Err(reason)
        }
    }
}

struct SchedulerSvc {
    nodes: Vec<NodeCfg>,
    policy: Policy,
    weights: Weights,
    opa: Option<Opa>,
    probe_timeout: Duration,
    /// EWMA of past success per (run_id, node). Keyed by run so that
    /// repetitions are independent.
    avail: Arc<Mutex<HashMap<(String, String), f64>>>,
}

impl SchedulerSvc {
    async fn probe_all(&self) -> Vec<Option<(Profile, f64)>> {
        let mut set: JoinSet<(usize, Option<(Profile, f64)>)> = JoinSet::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let ch = n.channel.clone();
            let to = self.probe_timeout;
            set.spawn(async move {
                let mut c = ParticipantClient::new(ch);
                let t0 = Instant::now();
                let r = tokio::time::timeout(to, c.get_profile(Empty {})).await;
                let rtt_ms = t0.elapsed().as_secs_f64() * 1000.0;
                match r {
                    Ok(Ok(resp)) => (i, Some((resp.into_inner(), rtt_ms))),
                    _ => (i, None),
                }
            });
        }
        let mut out: Vec<Option<(Profile, f64)>> = vec![None; self.nodes.len()];
        while let Some(j) = set.join_next().await {
            if let Ok((i, r)) = j {
                out[i] = r;
            }
        }
        out
    }

    fn views(&self, run_id: &str, probes: &[Option<(Profile, f64)>]) -> Vec<NodeView> {
        let avail = self.avail.lock().unwrap();
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| match &probes[i] {
                Some((p, rtt)) => NodeView {
                    id: n.id.clone(),
                    reachable: true,
                    jurisdiction: p.jurisdiction.clone(),
                    license_active: p.license_status == "active",
                    residency_zone: p.residency_zone.clone(),
                    bench_mops: p.bench_mops,
                    bandwidth_mbps: p.bandwidth_mbps as f64,
                    rtt_ms: *rtt,
                    availability: *avail.get(&(run_id.to_string(), n.id.clone())).unwrap_or(&1.0),
                },
                None => NodeView {
                    id: n.id.clone(),
                    reachable: false,
                    jurisdiction: String::new(),
                    license_active: false,
                    residency_zone: String::new(),
                    bench_mops: 0.0,
                    bandwidth_mbps: 0.0,
                    rtt_ms: 0.0,
                    availability: 0.0,
                },
            })
            .collect()
    }
}

#[tonic::async_trait]
impl Scheduler for SchedulerSvc {
    async fn select(&self, request: Request<SelectRequest>) -> Result<Response<SelectResponse>, Status> {
        let r = request.into_inner();
        let strategy = Strategy::parse(&r.strategy)
            .ok_or_else(|| Status::invalid_argument(format!("unknown strategy '{}'", r.strategy)))?;

        // The random baseline (plain FedAvg client sampling) does NOT probe.
        let probes: Vec<Option<(Profile, f64)>> = if strategy == Strategy::Random {
            vec![None; self.nodes.len()]
        } else {
            self.probe_all().await
        };
        let mut views = self.views(&r.run_id, &probes);
        if strategy == Strategy::Random {
            for v in views.iter_mut() {
                v.reachable = true; // random does not know otherwise
            }
        }

        // Policy verdicts: native reference always; OPA (authoritative) if configured.
        let mut verdicts: Vec<Result<(), String>> = views
            .iter()
            .map(|v| if v.reachable { self.policy.check(v) } else { Err("unreachable".to_string()) })
            .collect();
        let mut disagreements = 0u32;
        if strategy == Strategy::PolicyResource {
            if let Some(opa) = &self.opa {
                for (i, pr) in probes.iter().enumerate() {
                    if let Some((p, _)) = pr {
                        let ov = opa.verdict(p).await;
                        if ov.is_ok() != verdicts[i].is_ok() {
                            disagreements += 1;
                        }
                        verdicts[i] = ov;
                    }
                }
            }
        }

        let mut rng = SplitMix64::new(derive_seed(r.seed, &format!("{}#{}", r.run_id, r.round)));
        let sel = select_nodes(&views, &verdicts, r.k as usize, strategy, &self.weights, &mut rng);

        let selected: Vec<Candidate> = sel
            .selected
            .iter()
            .map(|(i, score)| {
                let profile = probes[*i]
                    .as_ref()
                    .map(|(p, _)| p.clone())
                    .unwrap_or_else(|| Profile { id: self.nodes[*i].id.clone(), ..Default::default() });
                Candidate {
                    id: self.nodes[*i].id.clone(),
                    endpoint: self.nodes[*i].endpoint.clone(),
                    profile: Some(profile),
                    rtt_ms: views[*i].rtt_ms,
                    score: *score,
                }
            })
            .collect();
        let excluded: Vec<Exclusion> = sel
            .excluded
            .iter()
            .map(|(i, reason)| Exclusion { id: self.nodes[*i].id.clone(), reason: reason.clone() })
            .collect();

        println!(
            "{{\"ev\":\"select\",\"run\":\"{}\",\"round\":{},\"strategy\":\"{}\",\"selected\":{},\"excluded\":{},\"policy_disagreements\":{}}}",
            r.run_id,
            r.round,
            strategy.as_str(),
            selected.len(),
            excluded.len(),
            disagreements
        );

        Ok(Response::new(SelectResponse { selected, excluded, policy_disagreements: disagreements }))
    }

    async fn report_outcome(&self, request: Request<OutcomeReport>) -> Result<Response<Empty>, Status> {
        let r = request.into_inner();
        let mut avail = self.avail.lock().unwrap();
        let a = avail.entry((r.run_id, r.participant_id)).or_insert(1.0);
        *a = 0.7 * *a + 0.3 * if r.ok { 1.0 } else { 0.0 };
        Ok(Response::new(Empty {}))
    }
}

fn csv(key: &str, default: &str) -> Vec<String> {
    env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn env_f64(key: &str, default: f64) -> f64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // PARTICIPANTS="n1=host1:50052,n2=host2:50052,..."
    let spec = env::var("PARTICIPANTS").expect("PARTICIPANTS env var is required, e.g. n1=host:50052,n2=host:50052");
    let probe_timeout = Duration::from_millis(env_f64("PROBE_TIMEOUT_MS", 1500.0) as u64);

    let mut nodes = Vec::new();
    for item in spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let (id, endpoint) = item.split_once('=').expect("PARTICIPANTS entries must look like id=host:port");
        let channel = Endpoint::from_shared(format!("http://{endpoint}"))?
            .connect_timeout(Duration::from_millis(1000))
            .connect_lazy();
        nodes.push(NodeCfg { id: id.to_string(), endpoint: endpoint.to_string(), channel });
    }

    let policy = Policy {
        allowed_jurisdictions: csv("ALLOWED_JURISDICTIONS", "UG"),
        allowed_zones: csv("ALLOWED_ZONES", "ug-central"),
    };
    let d = Weights::default();
    let weights = Weights {
        compute: env_f64("W_COMPUTE", d.compute),
        bandwidth: env_f64("W_BANDWIDTH", d.bandwidth),
        latency: env_f64("W_LATENCY", d.latency),
        availability: env_f64("W_AVAILABILITY", d.availability),
    };

    let backend = env::var("POLICY_BACKEND").unwrap_or_else(|_| "native".to_string());
    let opa = if backend == "opa" {
        let url = env::var("OPA_URL").unwrap_or_else(|_| "http://opa:8181".to_string());
        let http = reqwest::Client::builder().timeout(Duration::from_millis(1000)).build()?;
        Some(Opa { url, http })
    } else {
        None
    };

    println!(
        "{{\"ev\":\"start\",\"nodes\":{},\"policy_backend\":\"{}\",\"allowed_jurisdictions\":{:?},\"allowed_zones\":{:?}}}",
        nodes.len(),
        backend,
        policy.allowed_jurisdictions,
        policy.allowed_zones
    );

    let svc = SchedulerSvc { nodes, policy, weights, opa, probe_timeout, avail: Arc::new(Mutex::new(HashMap::new())) };
    let addr: SocketAddr = "0.0.0.0:50051".parse()?;
    Server::builder().add_service(SchedulerServer::new(svc)).serve(addr).await?;
    Ok(())
}
