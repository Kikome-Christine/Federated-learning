//! Regulatory-aware, resource-aware participant scheduler (paper Sec. III-D).
//!
//! Every round it (1) probes every node's profile, (2) evaluates residency
//! policy via native checks and an Open Policy Agent sidecar, (3) scores and
//! selects using the operational weights. Policy is enforced continuously.

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

// --- Environment and Parsing Helpers ---

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn parse_csv_env(key: &str, default: &str) -> Vec<String> {
    env_str(key, default)
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// --- Data Structures ---

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
    /// Fail-closed: any error is treated as "deny" to protect cross-border data constraints.
    /// Maps directly to Section 19 of the Uganda Data Protection and Privacy Act (2019).
    async fn verdict(&self, p: &Profile) -> Result<(), String> {
        let has_valid_attestation = !p.id.is_empty(); 
        let attestation_is_stale = p.license_status == "stale";

        let body = serde_json::json!({
            "input": {
                "jurisdiction": p.jurisdiction,
                "residency_zone": p.residency_zone,
                "license_status": p.license_status,
                "has_valid_attestation": has_valid_attestation,
                "attestation_is_stale": attestation_is_stale,
            }
        });

        let resp = self
            .http
            .post(format!("{}/v1/data/fl/residency", self.url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("opa_unreachable:{}", e))?;

        let v: serde_json::Value = resp.json().await.map_err(|e| format!("opa_bad_response:{}", e))?;
        
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

        let probes: Vec<Option<(Profile, f64)>> = if strategy == Strategy::Random {
            vec![None; self.nodes.len()]
        } else {
            self.probe_all().await
        };
        let mut views = self.views(&r.run_id, &probes);
        if strategy == Strategy::Random {
            for v in views.iter_mut() {
                v.reachable = true;
            }
        }

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
            r.run_id, r.round, strategy.as_str(), selected.len(), excluded.len(), disagreements
        );

        Ok(Response::new(SelectResponse { selected, excluded, policy_disagreements: disagreements }))
    }

    async fn report_outcome(&self, request: Request<OutcomeReport>) -> Result<Response<Empty>, Status> {
        let r = request.into_inner();
        let mut avail = self.avail.lock().unwrap();
        
        let alpha = 0.2;
        let key = (r.run_id.clone(), r.participant_id.clone());
        let current = avail.entry(key).or_insert(1.0);

        if r.ok {
            *current = (1.0 - alpha) * (*current) + alpha * 1.0;
        } else {
            *current = (1.0 - alpha) * (*current) + alpha * 0.0;
        }
        
        Ok(Response::new(Empty {}))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend_type = env_str("POLICY_BACKEND", "opa");
    let opa_url = env_str("OPA_URL", "http://opa:8181");
    
    let w_compute = env_or("W_COMPUTE", 0.35);
    let w_bandwidth = env_or("W_BANDWIDTH", 0.25);
    let w_latency = env_or("W_LATENCY", 0.25);
    let w_availability = env_or("W_AVAILABILITY", 0.15);

    let weights = Weights {
        compute: w_compute,
        bandwidth: w_bandwidth,
        latency: w_latency,
        availability: w_availability,
    };

    let policy = Policy {
        allowed_jurisdictions: parse_csv_env("ALLOWED_JURISDICTIONS", "UG"),
        allowed_zones: parse_csv_env("ALLOWED_ZONES", "ug-central"),
    };

    let opa = if backend_type == "opa" {
        println!("Authoritative security plane activated via OPA container at: {}", opa_url);
        Some(Opa {
            url: opa_url,
            http: reqwest::Client::new(),
        })
    } else {
        None
    };

    let raw_participants = env_str("PARTICIPANTS", "");
    let mut nodes = Vec::new();

    for item in raw_participants.split(',').filter(|s| !s.is_empty()) {
        if let Some((id, endpoint)) = item.split_once('=') {
            let target_endpoint = if endpoint.contains("://") {
                endpoint.to_string()
            } else {
                format!("http://{}", endpoint)
            };
            
            // Pass by clone configuration to satisfy standard move constraints
            let ep = Endpoint::from_shared(target_endpoint.clone())?
                .connect_timeout(Duration::from_millis(1500));
            let channel = ep.connect_lazy();

            nodes.push(NodeCfg {
                id: id.trim().to_string(),
                endpoint: endpoint.trim().to_string(),
                channel,
            });
        }
    }

    let probe_timeout_ms = env_or("PROBE_TIMEOUT_MS", 1500);
    let svc = SchedulerSvc {
        nodes,
        policy,
        weights,
        opa,
        probe_timeout: Duration::from_millis(probe_timeout_ms),
        avail: Arc::new(Mutex::new(HashMap::new())),
    };

    let addr: SocketAddr = "0.0.0.0:50051".parse()?;
    println!("Scheduler microservice listening on bound port: {}", addr);
    
    Server::builder()
        .add_service(SchedulerServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}




