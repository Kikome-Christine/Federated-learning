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
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flcommon::model::{self, fedavg};
use flcommon::rng::{derive_seed, SplitMix64};
use flproto::fl::participant_client::ParticipantClient;
use flproto::fl::scheduler_client::SchedulerClient;
use flproto::fl::{OutcomeReport, SelectRequest, TrainRequest, TrainResponse};
use hdrhistogram::Histogram;
use prost::Message;
use serde_json::{json, Value};
use tokio::task::JoinSet;
use tonic::transport::{Channel, Endpoint};

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
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
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
    data_seed: u64,
    eval_seed: u64,
    base_seed: u64,
    tier: String,
    substrate: String,
    exp: String,
    cond_id: String,
    cond_json: Value,
    ineligible: Vec<String>,
    node_ids: Vec<String>,
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
            data_seed: env_or("DATA_SEED", 7),
            eval_seed: env_or("EVAL_SEED", 424242),
            base_seed: env_or("BASE_SEED", 1000),
            tier: env_str("TIER", "emu"),
            substrate: env_str("SUBSTRATE", "compose"),
            exp: env_str("EXP", "adhoc"),
            cond_id: env_str("COND_ID", "default"),
            cond_json,
            ineligible: csv("INELIGIBLE_IDS", ""),
            node_ids: csv("NODE_IDS", ""),
            out_dir: env_str("OUT_DIR", "/results/raw"),
            label: env_str("RUN_LABEL", "run"),
        }
    }

    /// Fields common to every record, so the analysis needs no side-channel metadata.
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
    let ch = Endpoint::from_shared(format!("http://{endpoint}"))
        .map_err(|e| format!("bad endpoint {endpoint}: {e}"))?
        .connect_timeout(Duration::from_millis(1000))
        .connect_lazy();
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
    // microseconds, 1us .. 1h, 3 significant digits
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

#[allow(clippy::too_many_arguments)]
async fn run_one(
    cfg: &Cfg,
    sched: &mut SchedulerClient<Channel>,
    clients: &mut HashMap<String, ParticipantClient<Channel>>,
    eval: &model::Dataset,
    rep: u32,
    order_index: usize,
    strategy: &str,
    out: &mut File,
) -> Result<(), BoxErr> {
    let run_id = format!("{}-{}-{}-rep{}", cfg.label, cfg.cond_id, strategy, rep);
    let seed = cfg.base_seed + rep as u64;
    let mut global = vec![0.0f32; cfg.dim + 1];
    let mut hist = new_hist()?;

    for round in 0..cfg.rounds {
        let t_unix_ms = now_ms();
        let t0 = Instant::now();

        // ---- 1. selection (includes probing + policy evaluation)
        let sel_req = SelectRequest { run_id: run_id.clone(), round, k: cfg.k, strategy: strategy.to_string(), seed };
        let sel = match tokio::time::timeout(cfg.select_timeout, sched.select(sel_req)).await {
            Ok(Ok(r)) => Some(r.into_inner()),
            _ => None,
        };
        let select_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let mut rec = cfg.base_record("round", strategy, rep);
        rec.insert("order_index".into(), json!(order_index));
        rec.insert("round".into(), json!(round));
        rec.insert("warmup".into(), json!(round < cfg.warmup));
        rec.insert("t_unix_ms".into(), json!(t_unix_ms));

        let sel = match sel {
            Some(s) => s,
            None => {
                rec.insert("success".into(), json!(false));
                rec.insert("select_error".into(), json!(true));
                rec.insert("round_ms".into(), json!(t0.elapsed().as_secs_f64() * 1000.0));
                write_line(out, Value::Object(rec))?;
                continue;
            }
        };

        // ---- 2. parallel local training, each bounded by the round deadline
        let mut set: JoinSet<(String, Result<TrainResponse, String>)> = JoinSet::new();
        let mut bytes_down = 0u64;
        for c in &sel.selected {
            let mut client = match client_for(clients, &c.endpoint) {
                Ok(cl) => cl,
                Err(_) => continue,
            };
            let req = TrainRequest {
                run_id: run_id.clone(),
                round,
                local_epochs: cfg.local_epochs,
                learning_rate: cfg.lr,
                global_weights: global.clone(),
            };
            bytes_down += req.encoded_len() as u64;
            let id = c.id.clone();
            let deadline = cfg.deadline;
            set.spawn(async move {
                let res = match tokio::time::timeout(deadline, client.train(req)).await {
                    Ok(Ok(r)) => Ok(r.into_inner()),
                    Ok(Err(s)) => Err(format!("rpc_{:?}", s.code())),
                    Err(_) => Err("deadline".to_string()),
                };
                (id, res)
            });
        }

        let mut ok: Vec<TrainResponse> = Vec::new();
        let mut failed: Vec<(String, String)> = Vec::new();
        while let Some(j) = set.join_next().await {
            match j {
                Ok((_id, Ok(r))) => ok.push(r),
                Ok((id, Err(e))) => failed.push((id, e)),
                Err(e) => failed.push(("join".into(), format!("join_error:{e}"))),
            }
        }
        ok.sort_by(|a, b| a.participant_id.cmp(&b.participant_id)); // deterministic FedAvg order
        let bytes_up: u64 = ok.iter().map(|r| r.encoded_len() as u64).sum();

        // ---- 3. aggregation (only if enough participants answered)
        let mut success = false;
        if ok.len() >= cfg.min_participants {
            let ups: Vec<(Vec<f32>, u64)> = ok.iter().map(|r| (r.weights.clone(), r.num_samples)).collect();
            if let Some(g) = fedavg(&ups) {
                global = g;
                success = true;
            }
        }
        let round_ms = t0.elapsed().as_secs_f64() * 1000.0; // round completion latency
        if round >= cfg.warmup {
            hist.record(((round_ms * 1000.0) as u64).max(1)).ok();
        }

        // ---- 4. feed outcomes back to the scheduler (paper's Recovery/Learning stage)
        for r in &ok {
            let _ = sched
                .report_outcome(OutcomeReport { run_id: run_id.clone(), participant_id: r.participant_id.clone(), ok: true })
                .await;
        }
        for (id, _) in &failed {
            let _ = sched
                .report_outcome(OutcomeReport { run_id: run_id.clone(), participant_id: id.clone(), ok: false })
                .await;
        }

        // ---- 5. evaluation (outside the timed region)
        let global_loss = model::eval_loss(eval, &global);

        rec.insert(
            "selected".into(),
            json!(sel
                .selected
                .iter()
                .map(|c| json!({
                    "id": c.id,
                    "rtt_ms": c.rtt_ms,
                    "score": c.score,
                    "bench_mops": c.profile.as_ref().map(|p| p.bench_mops).unwrap_or(0.0),
                }))
                .collect::<Vec<_>>()),
        );
        rec.insert(
            "excluded".into(),
            json!(sel.excluded.iter().map(|e| json!({"id": e.id, "reason": e.reason})).collect::<Vec<_>>()),
        );
        rec.insert(
            "responded".into(),
            json!(ok
                .iter()
                .map(|r| {
                    let a = r.attestation.clone().unwrap_or_default();
                    json!({
                        "id": r.participant_id,
                        "train_us": r.train_us,
                        "loss": r.local_loss,
                        "jurisdiction": a.jurisdiction,
                        "license": a.license_status,
                        "zone": a.residency_zone,
                    })
                })
                .collect::<Vec<_>>()),
        );
        rec.insert("failed".into(), json!(failed.iter().map(|(id, e)| json!({"id": id, "err": e})).collect::<Vec<_>>()));
        rec.insert("success".into(), json!(success));
        rec.insert("select_ms".into(), json!(select_ms));
        rec.insert("round_ms".into(), json!(round_ms));
        rec.insert("bytes_down".into(), json!(bytes_down));
        rec.insert("bytes_up".into(), json!(bytes_up));
        rec.insert("global_loss".into(), json!(global_loss));
        rec.insert("policy_disagreements".into(), json!(sel.policy_disagreements));
        write_line(out, Value::Object(rec))?;
    }

    write_line(out, summary_record(cfg, strategy, rep, &hist))?;
    Ok(())
}

/// Naive floor baseline: pool everything, train in one process.
fn run_centralized(cfg: &Cfg, eval: &model::Dataset, out: &mut File) -> Result<(), BoxErr> {
    if cfg.node_ids.is_empty() {
        return Err("MODE=centralized requires NODE_IDS (comma-separated institution ids)".into());
    }
    for rep in 0..cfg.reps {
        let t_setup = Instant::now();
        let mut xs: Vec<f32> = Vec::new();
        let mut ys: Vec<u8> = Vec::new();
        let mut n = 0usize;
        for id in &cfg.node_ids {
            let d = model::institution_dataset(cfg.data_seed, id, cfg.n_samples, cfg.dim);
            xs.extend_from_slice(&d.x);
            ys.extend_from_slice(&d.y);
            n += d.n;
        }
        let pooled = model::Dataset { x: xs, y: ys, n, dim: cfg.dim };
        let setup_ms = t_setup.elapsed().as_secs_f64() * 1000.0;

        let mut global = vec![0.0f32; cfg.dim + 1];
        let mut hist = new_hist()?;
        for round in 0..cfg.rounds {
            let t_unix_ms = now_ms();
            let t0 = Instant::now();
            model::train_local(&pooled, &mut global, cfg.local_epochs, cfg.lr);
            let round_ms = t0.elapsed().as_secs_f64() * 1000.0;
            if round >= cfg.warmup {
                hist.record(((round_ms * 1000.0) as u64).max(1)).ok();
            }
            let loss = model::eval_loss(eval, &global);

            let mut rec = cfg.base_record("round", "centralized", rep);
            rec.insert("order_index".into(), json!(0));
            rec.insert("round".into(), json!(round));
            rec.insert("warmup".into(), json!(round < cfg.warmup));
            rec.insert("t_unix_ms".into(), json!(t_unix_ms));
            rec.insert("selected".into(), json!(cfg.node_ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()));
            rec.insert("excluded".into(), json!([]));
            // Every institution's RAW data is pooled => every ineligible node's data left its boundary.
            rec.insert("responded".into(), json!(cfg.node_ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()));
            rec.insert("failed".into(), json!([]));
            rec.insert("success".into(), json!(true));
            rec.insert("select_ms".into(), json!(0.0));
            rec.insert("round_ms".into(), json!(round_ms));
            rec.insert("bytes_down".into(), json!(0));
            rec.insert("bytes_up".into(), json!(0));
            rec.insert("global_loss".into(), json!(loss));
            rec.insert("setup_ms".into(), json!(setup_ms));
            write_line(out, Value::Object(rec))?;
        }
        write_line(out, summary_record(cfg, "centralized", rep, &hist))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), BoxErr> {
    let cfg = Cfg::from_env();
    fs::create_dir_all(&cfg.out_dir)?;
    let path = format!("{}/{}.jsonl", cfg.out_dir, cfg.label);
    let mut out = OpenOptions::new().create(true).append(true).open(&path)?;

    let mut meta = cfg.base_record("meta", "", 0);
    meta.insert("t_unix_ms".into(), json!(now_ms()));
    meta.insert("git_sha".into(), json!(env_str("GIT_SHA", "unknown")));
    meta.insert("mode".into(), json!(cfg.mode));
    meta.insert("strategies".into(), json!(cfg.strategies));
    meta.insert("reps".into(), json!(cfg.reps));
    meta.insert("rounds".into(), json!(cfg.rounds));
    meta.insert("warmup_rounds".into(), json!(cfg.warmup));
    meta.insert("k".into(), json!(cfg.k));
    meta.insert("min_participants".into(), json!(cfg.min_participants));
    meta.insert("deadline_ms".into(), json!(cfg.deadline.as_millis() as u64));
    meta.insert("model_dim".into(), json!(cfg.dim));
    meta.insert("n_samples".into(), json!(cfg.n_samples));
    meta.insert("local_epochs".into(), json!(cfg.local_epochs));
    meta.insert("base_seed".into(), json!(cfg.base_seed));
    meta.insert("host_cpus".into(), json!(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)));
    write_line(&mut out, Value::Object(meta))?;

    let eval = model::gen_dataset(cfg.eval_seed, cfg.n_eval, cfg.dim, 0.10);

    if cfg.mode == "centralized" {
        return run_centralized(&cfg, &eval, &mut out);
    }

    let sched_ch = Endpoint::from_shared(cfg.scheduler_url.clone())?.connect_timeout(Duration::from_secs(2)).connect_lazy();
    let mut sched = SchedulerClient::new(sched_ch);
    wait_for_scheduler(&cfg, &mut sched).await;

    let mut clients: HashMap<String, ParticipantClient<Channel>> = HashMap::new();
    for rep in 0..cfg.reps {
        let mut order = cfg.strategies.clone();
        SplitMix64::new(derive_seed(cfg.base_seed, &format!("order{rep}"))).shuffle(&mut order);
        for (oi, strategy) in order.iter().enumerate() {
            run_one(&cfg, &mut sched, &mut clients, &eval, rep, oi, strategy, &mut out).await?;
        }
        eprintln!("rep {}/{} done", rep + 1, cfg.reps);
    }
    Ok(())
}
