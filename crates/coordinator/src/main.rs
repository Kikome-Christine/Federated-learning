//! Experiment driver / FL coordinator.
//!
//! One invocation runs REPS independent repetitions of ROUNDS federated
//! rounds for each strategy in STRATEGIES. Within every repetition the
//! strategy order is shuffled from the seed (interleaving cancels slow drift
//! in the host / network). Every round is written as one JSON line, so the
//! analysis (analysis/analyze.py) sees the raw data, not just summaries.
//!
//! MODE=centralized is the naive floor baseline: all institutions' data is
//! pooled and trained in a single process (no distribution, no residency).

use std::{
    collections::HashMap,
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flcommon::model::{self, fedavg, Dataset};
use flcommon::rng::{derive_seed, SplitMix64};
use flproto::fl::participant_client::ParticipantClient;
use flproto::fl::scheduler_client::SchedulerClient;
use flproto::fl::{OutcomeReport, SelectRequest, TrainRequest, TrainResponse};
use hdrhistogram::Histogram;
use serde_json::{json, Value};
use tokio::task::JoinSet;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

type BoxErr = Box<dyn std::error::Error>;

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}
fn env_or<T: FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn csv(key: &str, default: &str) -> Vec<String> {
    env_str(key, default).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

struct Cfg {
    scheduler_url: String,
    mode: String,
    strategies: Vec<String>,
    reps: u32,
    rounds: u32,
    warmup: u32,
    k: u32,
    min_participants: usize,
    deadline: Duration,
    select_timeout: Duration,
    local_epochs: u32,
    lr: f32,
    dim: usize,
    n_samples: usize,
    n_eval: usize,
    eval_seed: u64,
    base_seed: u64,
    tier: String,
    substrate: String,
    exp: String,
    cond_id: String,
    cond_json: Value,
    ineligible: Vec<String>,
    out_dir: String,
    label: String,
}

impl Cfg {
    fn from_env() -> Cfg {
        let cond_json: Value = serde_json::from_str(&env_str("COND_JSON", "{}")).unwrap_or_else(|_| json!({}));
        Cfg {
            scheduler_url: env_str("SCHEDULER_URL", "http://scheduler:50051"),
            mode: env_str("MODE", "federated"),
            strategies: csv("STRATEGIES", "random,resource_only,policy_resource"),
            reps: env_or("REPS", 30),
            rounds: env_or("ROUNDS", 20),
            warmup: env_or("WARMUP_ROUNDS", 2),
            k: env_or("K", 4),
            min_participants: env_or("MIN_PARTICIPANTS", 3),
            deadline: Duration::from_millis(env_or("DEADLINE_MS", 5000)),
            select_timeout: Duration::from_millis(env_or("SELECT_TIMEOUT_MS", 8000)),
            local_epochs: env_or("LOCAL_EPOCHS", 3),
            lr: env_or("LEARNING_RATE", 0.5),
            dim: env_or("MODEL_DIM", 4096),
            n_samples: env_or("N_SAMPLES", 1000),
            n_eval: env_or("N_EVAL", 2000),
            eval_seed: env_or("EVAL_SEED", 424242),
            base_seed: env_or("BASE_SEED", 1000),
            tier: env_str("TIER", "emu"),
            substrate: env_str("SUBSTRATE", "compose"),
            exp: env_str("EXP", "adhoc"),
            cond_id: env_str("COND_ID", "default"),
            cond_json,
            ineligible: csv("INELIGIBLE_IDS", ""),
            out_dir: env_str("OUT_DIR", "/results/raw"),
            label: env_str("RUN_LABEL", "run"),
        }
    }

    fn base_record(&self, ty: &str, strategy: &str, rep: u32) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("type".into(), json!(ty));
        m.insert("label".into(), json!(self.label));
        m.insert("tier".into(), json!(self.tier));
        m.insert("substrate".into(), json!(self.substrate));
        m.insert("exp".into(), json!(self.exp));
        m.insert("cond_id".into(), json!(self.cond_id));
        m.insert("cond".into(), self.cond_json.clone());
        m.insert("strategy".into(), json!(strategy));
        m.insert("rep".into(), json!(rep));
        m.insert("ineligible".into(), json!(self.ineligible));
        m
    }
}

fn client_for(
    cache: &mut HashMap<String, ParticipantClient<Channel>>,
    endpoint: &str,
) -> Result<ParticipantClient<Channel>, String> {
    if let Some(c) = cache.get(endpoint) {
        return Ok(c.clone());
    }
    let url = if endpoint.contains("://") { endpoint.to_string() } else { format!("http://{endpoint}") };
    let mut ep = Endpoint::from_shared(url.clone())
        .map_err(|e| format!("bad endpoint {endpoint}: {e}"))?
        .connect_timeout(Duration::from_millis(1000));
    if url.starts_with("https://") {
        ep = ep
            .tls_config(ClientTlsConfig::new().with_enabled_roots())
            .map_err(|e| format!("tls config for {endpoint}: {e}"))?;
    }
    let ch = ep.connect_lazy();
    let c = ParticipantClient::new(ch);
    cache.insert(endpoint.to_string(), c.clone());
    Ok(c)
}

fn write_line(out: &mut File, rec: Value) -> Result<(), BoxErr> {
    writeln!(out, "{}", rec)?;
    out.flush()?;
    Ok(())
}

fn summary_record(cfg: &Cfg, strategy: &str, rep: u32, hist: &Histogram<u64>) -> Value {
    let mut m = cfg.base_record("run_summary", strategy, rep);
    m.insert("n_rounds".into(), json!(hist.len()));
    m.insert("mean_ms".into(), json!(hist.mean() / 1000.0));
    m.insert("p50_ms".into(), json!(hist.value_at_quantile(0.50) as f64 / 1000.0));
    m.insert("p95_ms".into(), json!(hist.value_at_quantile(0.95) as f64 / 1000.0));
    m.insert("p99_ms".into(), json!(hist.value_at_quantile(0.99) as f64 / 1000.0));
    m.insert("max_ms".into(), json!(hist.max() as f64 / 1000.0));
    Value::Object(m)
}

fn new_hist() -> Result<Histogram<u64>, BoxErr> {
    Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).map_err(|e| format!("hdrhistogram: {e:?}").into())
}

async fn wait_for_scheduler(cfg: &Cfg, sched: &mut SchedulerClient<Channel>) {
    for attempt in 0..90 {
        let req = SelectRequest {
            run_id: "readiness-probe".into(),
            round: 0,
            k: 1,
            strategy: "resource_only".into(),
            seed: 0,
        };
        if let Ok(Ok(r)) = tokio::time::timeout(Duration::from_secs(3), sched.select(req)).await {
            if !r.into_inner().selected.is_empty() {
                eprintln!("scheduler ready after {attempt} attempts ({})", cfg.scheduler_url);
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eprintln!("WARNING: scheduler/participants not ready after 90 s; continuing anyway");
}

fn evaluate_auprc(dataset: &Dataset, weights: &[f32]) -> f64 {
    let mut scores: Vec<(f64, f32)> = Vec::with_capacity(dataset.n);
    let features_per_sample = dataset.dim;

    for i in 0..dataset.n {
        let offset = i * features_per_sample;
        let sample_features = &dataset.x[offset..offset + features_per_sample];
        let mut activation = weights[features_per_sample] as f64;
        for (j, &feat) in sample_features.iter().enumerate() {
            activation += (feat as f64) * (weights[j] as f64);
        }
        let probability = 1.0 / (1.0 + (-activation).exp());
        scores.push((probability, dataset.y[i] as f32));
    }

    scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let total_positives = dataset.y.iter().filter(|&&label| label > 0).count() as f64;
    if total_positives == 0.0 {
        return 0.0;
    }

    let mut true_positives = 0.0;
    let mut false_positives = 0.0;
    let mut previous_recall = 0.0;
    let mut auprc = 0.0;

    for (_prob, label) in scores {
        if label > 0.5 {
            true_positives += 1.0;
        } else {
            false_positives += 1.0;
        }
        let current_precision = true_positives / (true_positives + false_positives);
        let current_recall = true_positives / total_positives;
        
        auprc += current_precision * (current_recall - previous_recall);
        previous_recall = current_recall;
    }

    auprc
}

fn evaluate_loss(dataset: &Dataset, weights: &[f32]) -> f64 {
    let mut total_loss = 0.0;
    let features_per_sample = dataset.dim;

    for i in 0..dataset.n {
        let offset = i * features_per_sample;
        let sample_features = &dataset.x[offset..offset + features_per_sample];
        let mut activation = weights[features_per_sample] as f64;
        for (j, &feat) in sample_features.iter().enumerate() {
            activation += (feat as f64) * (weights[j] as f64);
        }
        let probability = 1.0 / (1.0 + (-activation).exp());
        let bounded_prob = probability.clamp(1e-7, 1.0 - 1e-7);
        let label = dataset.y[i] as f64;
        total_loss -= label * bounded_prob.ln() + (1.0 - label) * (1.0 - bounded_prob).ln();
    }

    total_loss / (dataset.n as f64)
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    cfg: &Cfg,
    sched: &mut SchedulerClient<Channel>,
    clients: &mut HashMap<String, ParticipantClient<Channel>>,
    eval: &model::Dataset,
    rep: u32,
    _order_index: usize,
    strategy: &str,
    out: &mut File,
) -> Result<(), BoxErr> {
    let run_id = format!("{}_rep_{:02}_{}", cfg.label, rep, strategy);
    let mut global_weights = vec![0.0f32; cfg.dim + 1];
    let mut round_hist = new_hist()?;

    let single_payload_bytes = (cfg.dim + 1) * std::mem::size_of::<f32>();
    let deadline_duration = cfg.deadline; // Fixes lifetime escape constraint

    for round in 1..=cfg.rounds {
        let t_round_start = Instant::now();
        let seed = derive_seed(cfg.base_seed, &format!("rep_{}_round_{}", rep, round));

        let select_req = SelectRequest {
            run_id: run_id.clone(),
            round: round as u32,
            k: cfg.k as u32,
            strategy: strategy.to_string(),
            seed,
        };

        let t_select = Instant::now();
        let select_res = match tokio::time::timeout(cfg.select_timeout, sched.select(select_req)).await {
            Ok(Ok(res)) => res.into_inner(),
            Ok(Err(e)) => {
                eprintln!("Round {round} selection protocol RPC exception: {e}");
                continue;
            }
            Err(_) => {
                eprintln!("Round {round} selection timed out beyond budget thresholds.");
                continue;
            }
        };
        let select_latency_us = t_select.elapsed().as_micros() as u64;

        if select_res.selected.is_empty() || select_res.selected.len() < cfg.min_participants {
            eprintln!("Round {round} aborted: insufficient compliant nodes selected (Got {}).", select_res.selected.len());
            continue;
        }

        let mut tasks = JoinSet::new();
        for node in &select_res.selected {
            let mut client = match client_for(clients, &node.endpoint) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed link connection to node {}: {}", node.id, e);
                    continue;
                }
            };

            let train_req = TrainRequest {
                run_id: run_id.clone(),
                round: round as u32,
                global_weights: global_weights.clone(),
                local_epochs: cfg.local_epochs as u32,
                learning_rate: cfg.lr,
            };

            tasks.spawn(async move {
                tokio::time::timeout(deadline_duration, client.train(train_req)).await
            });
        }

        let mut collected_updates = Vec::with_capacity(select_res.selected.len());
        while let Some(res) = tasks.join_next().await {
            if let Ok(Ok(Ok(response))) = res {
                collected_updates.push(response.into_inner());
            }
        }

        if collected_updates.len() < cfg.min_participants {
            eprintln!("Round {round} skipped: too many dropouts / stragglers timed out.");
            continue;
        }

        // Fixed parameter typing: explicitly bounds f32 vectors to match flcommon definitions
        let fedavg_tuple: Vec<(Vec<f32>, u64)> = collected_updates
            .iter()
            .map(|update| (update.weights.clone(), update.num_samples as u64))
            .collect();

        if let Some(new_weights) = fedavg(&fedavg_tuple) {
            global_weights = new_weights;
        }

        let global_loss = evaluate_loss(eval, &global_weights);
        let global_auprc = evaluate_auprc(eval, &global_weights);

        let round_latency_us = t_round_start.elapsed().as_micros() as u64;
        let total_wire_bytes = collected_updates.len() * 2 * single_payload_bytes;

        if round > cfg.warmup {
            let _ = round_hist.record(round_latency_us);
        }

        let record = json!({
            "type": "round_log",
            "run_id": run_id,
            "round": round,
            "round_latency_ms": round_latency_us as f64 / 1000.0,
            "select_us": select_latency_us,
            "active_cohort_size": collected_updates.len(),
            "wire_payload_bytes": total_wire_bytes,
            "global_loss": global_loss,
            "global_auprc": global_auprc,
            "policy_disagreements": select_res.policy_disagreements,
        });
        write_line(out, record)?;
    }

    if round_hist.len() > 0 {
        write_line(out, summary_record(cfg, strategy, rep, &round_hist))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), BoxErr> {
    let cfg = Cfg::from_env();
    fs::create_dir_all(&cfg.out_dir)?;
    let out_path = Path::new(&cfg.out_dir).join(format!("{}.jsonl", cfg.label));
    let mut out_file = OpenOptions::new().create(true).append(true).open(out_path)?;
    
    let eval_dataset = model::institution_dataset(cfg.eval_seed, "evaluator", cfg.n_eval, cfg.dim);
    
    let sched_channel = Endpoint::from_shared(cfg.scheduler_url.clone())?
        .connect_timeout(Duration::from_millis(2000))
        .connect().await?;
    let mut scheduler = SchedulerClient::new(sched_channel);
    
    let mut clients_cache = HashMap::new();
    wait_for_scheduler(&cfg, &mut scheduler).await;
    
    let mut rng = SplitMix64::new(cfg.base_seed);
    for rep in 1..=cfg.reps {
        let mut current_strategies = cfg.strategies.clone();
        for i in (1..current_strategies.len()).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            current_strategies.swap(i, j);
        }
        for (idx, strategy) in current_strategies.iter().enumerate() {
            if cfg.mode == "centralized" && strategy != "random" {
                continue;
            }
            run_one(
                &cfg,
                &mut scheduler,
                &mut clients_cache,
                &eval_dataset,
                rep,
                idx,
                strategy,
                &mut out_file,
            ).await?;
        }
    }
    Ok(())
}

