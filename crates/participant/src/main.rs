//! A federation participant (an institution's edge or cloud node).
//!
//! Heterogeneity is REAL, not simulated with sleep(): the container's CPU is
//! throttled by cgroups (docker `cpus:` / k8s limits) and the node MEASURES
//! its own speed at start-up. Training is real logistic-regression compute on
//! a seeded synthetic dataset and the update payload is a real weight vector.

use std::{env, net::SocketAddr, path::Path, str::FromStr, sync::Arc, time::{Duration, Instant}};

use flcommon::model::{self, Dataset};
use flproto::fl::participant_server::{Participant, ParticipantServer};
use flproto::fl::{Empty, Profile, TrainRequest, TrainResponse};
use tonic::{transport::Server, Request, Response, Status};

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_or<T: FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Measure sustained single-thread float throughput (million mul-adds/s) for
/// ~300 ms. Under a cgroup CPU quota this reflects the throttled speed.
fn bench_mops() -> f64 {
    let n = 1_000_000usize;
    let a: Vec<f32> = (0..n).map(|i| (i % 97) as f32 * 0.01).collect();
    let start = Instant::now();
    let mut acc = 0.0f32;
    let mut iters = 0u64;
    while start.elapsed() < Duration::from_millis(300) {
        for (x, y) in a.iter().zip(a.iter().skip(1)) {
            acc += x * y;
        }
        iters += 1;
    }
    std::hint::black_box(acc);
    (iters as f64 * n as f64) / start.elapsed().as_secs_f64() / 1e6
}

struct Svc {
    id: String,
    tier: String,
    jurisdiction: String,
    zone: String,
    cpu_cores: u32,
    memory_mb: u32,
    bandwidth_mbps: u32,
    bench_mops: f64,
    data: Arc<Dataset>,
    revoke_flag: String,
}

impl Svc {
    /// Current profile. A licence revocation is injected externally by
    /// creating the flag file (`docker exec <node> touch /tmp/revoked`), which
    /// lets experiments test continuous (per-round) policy enforcement.
    fn profile(&self) -> Profile {
        let license = if Path::new(&self.revoke_flag).exists() { "revoked" } else { "active" };
        Profile {
            id: self.id.clone(),
            tier: self.tier.clone(),
            jurisdiction: self.jurisdiction.clone(),
            license_status: license.to_string(),
            residency_zone: self.zone.clone(),
            cpu_cores: self.cpu_cores,
            memory_mb: self.memory_mb,
            bandwidth_mbps: self.bandwidth_mbps,
            bench_mops: self.bench_mops,
        }
    }
}

#[tonic::async_trait]
impl Participant for Svc {
    async fn get_profile(&self, _request: Request<Empty>) -> Result<Response<Profile>, Status> {
        Ok(Response::new(self.profile()))
    }

    async fn train(&self, request: Request<TrainRequest>) -> Result<Response<TrainResponse>, Status> {
        let req = request.into_inner();
        let dim = self.data.dim;
        if req.global_weights.len() != dim + 1 {
            return Err(Status::invalid_argument(format!(
                "expected {} weights, got {}",
                dim + 1,
                req.global_weights.len()
            )));
        }
        let epochs = req.local_epochs.max(1);
        let lr = req.learning_rate;
        let run_id = req.run_id.clone();
        let round = req.round;
        let mut w = req.global_weights;
        let data = Arc::clone(&self.data);

        let t0 = Instant::now();
        let (w, loss) = tokio::task::spawn_blocking(move || {
            let loss = model::train_local(&data, &mut w, epochs, lr);
            (w, loss)
        })
        .await
        .map_err(|e| Status::internal(format!("training task failed: {e}")))?;
        let train_us = t0.elapsed().as_micros() as u64;

        let attestation = self.profile();
        // Independent audit trail: one structured line per training event.
        println!(
            "{{\"ev\":\"train\",\"id\":\"{}\",\"run\":\"{}\",\"round\":{},\"train_us\":{},\"jurisdiction\":\"{}\",\"license\":\"{}\"}}",
            self.id, run_id, round, train_us, attestation.jurisdiction, attestation.license_status
        );

        Ok(Response::new(TrainResponse {
            participant_id: self.id.clone(),
            weights: w,
            num_samples: self.data.n as u64,
            local_loss: loss,
            train_us,
            attestation: Some(attestation),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = env_str("PARTICIPANT_ID", "unknown");
    let ineligible: Vec<String> = env_str("INELIGIBLE_IDS", "")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let is_ineligible = ineligible.contains(&id);

    // Ineligible nodes sit in a foreign jurisdiction / residency zone.
    let jurisdiction = if is_ineligible { env_str("ALT_JURISDICTION", "XX") } else { env_str("JURISDICTION", "UG") };
    let zone = if is_ineligible { env_str("ALT_ZONE", "xx-1") } else { env_str("RESIDENCY_ZONE", "ug-central") };

    let dim: usize = env_or("MODEL_DIM", 4096);
    let n_samples: usize = env_or("N_SAMPLES", 1000);
    let data_seed: u64 = env_or("DATA_SEED", 7);

    let bench = bench_mops();
    let data = Arc::new(model::institution_dataset(data_seed, &id, n_samples, dim));

    let svc = Svc {
        id: id.clone(),
        tier: env_str("TIER", "edge"),
        jurisdiction,
        zone,
        cpu_cores: env_or("CPU_CORES", 1),
        memory_mb: env_or("MEMORY_MB", 512),
        bandwidth_mbps: env_or("BANDWIDTH_MBPS", 50),
        bench_mops: bench,
        data,
        revoke_flag: env_str("REVOKE_FLAG", "/tmp/revoked"),
    };

    let p = svc.profile();
    println!(
        "{{\"ev\":\"start\",\"id\":\"{}\",\"tier\":\"{}\",\"jurisdiction\":\"{}\",\"zone\":\"{}\",\"bench_mops\":{:.1},\"dim\":{},\"n\":{}}}",
        p.id, p.tier, p.jurisdiction, p.residency_zone, p.bench_mops, dim, n_samples
    );

    let addr: SocketAddr = "0.0.0.0:50052".parse()?;
    Server::builder().add_service(ParticipantServer::new(svc)).serve(addr).await?;
    Ok(())
}
