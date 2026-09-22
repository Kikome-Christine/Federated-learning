//! flsim: deterministic, seeded simulator of the FL orchestration round.
//!
//! It runs the SAME selection code (flcommon::select) as the real scheduler
//! against a modelled network + CPU, on a VIRTUAL clock: identical seed and
//! arguments => byte-identical output (asserted by the unit tests below).
//! It emits the same JSONL schema as the real coordinator, so analysis is
//! tier-agnostic and simulated-vs-emulated-vs-production comparison is a
//! join on `cond_id`.
//!
//! Scope statement (put this in the paper): flsim is a purpose-built seeded
//! discrete simulator, not turmoil/madsim. It models per-node compute, link
//! delay, bandwidth caps and TCP-like loss recovery (RTO per loss event
//! round). It does not model congestion control, HTTP/2 flow control or
//! scheduler jitter -- exactly the effects the production tier can expose.
//! It does not train a model, so `global_loss` is null in the sim tier.

use std::{
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
};

use flcommon::rng::{derive_seed, SplitMix64};
use flcommon::select::{select, NodeView, Policy, Strategy, Weights};
use hdrhistogram::Histogram;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
struct ClassCfg {
    link_delay_ms: f64,
    link_rate_mbit: f64,
    bench_mops: f64,
}

#[derive(Clone, Debug)]
struct NodeCfg {
    id: String,
    class: ClassCfg,
}

struct Topo {
    dim: usize,
    n_samples: usize,
    local_epochs: u32,
    nodes: Vec<NodeCfg>,
    weights: Weights,
    policy: Policy,
    train_slowdown: f64,
    rto_min_ms: f64,
    jitter_ms: f64,
    probe_timeout_ms: f64,
}

struct SimCfg {
    label: String,
    tier: String,
    substrate: String,
    exp: String,
    cond_id: String,
    cond_json: Value,
    strategies: Vec<String>,
    reps: u32,
    rounds: u32,
    warmup: u32,
    k: usize,
    min_participants: usize,
    deadline_ms: f64,
    base_seed: u64,
    ineligible_ids: Vec<String>,
    ineligible_fraction: Option<f64>,
    down_ids: Vec<String>,
    loss_pct: f64,
    delay_ms: f64,
    rate_kbps: f64, // 0 => no cap
}

fn f(v: &Value, default: f64) -> f64 {
    v.as_f64().unwrap_or(default)
}

fn str_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn parse_topology(v: &Value) -> Result<Topo, String> {
    let m = &v["model"];
    let classes = v["classes"].as_object().ok_or("topology.classes missing")?;
    let mut nodes = Vec::new();
    for n in v["nodes"].as_array().ok_or("topology.nodes missing")? {
        let id = n["id"].as_str().ok_or("node.id missing")?.to_string();
        let cname = n["class"].as_str().ok_or("node.class missing")?;
        let c = classes.get(cname).ok_or_else(|| format!("unknown class {cname}"))?;
        nodes.push(NodeCfg {
            id,
            class: ClassCfg {
                link_delay_ms: f(&c["link_delay_ms"], 0.0),
                link_rate_mbit: f(&c["link_rate_mbit"], 100.0),
                bench_mops: f(&c["sim_bench_mops"], 1000.0),
            },
        });
    }
    let w = &v["scheduler"]["weights"];
    let d = Weights::default();
    let weights = Weights {
        compute: f(&w["compute"], d.compute),
        bandwidth: f(&w["bandwidth"], d.bandwidth),
        latency: f(&w["latency"], d.latency),
        availability: f(&w["availability"], d.availability),
    };
    let policy = Policy {
        allowed_jurisdictions: str_list(&v["policy"]["allowed_jurisdictions"]),
        allowed_zones: str_list(&v["policy"]["allowed_zones"]),
    };
    let s = &v["sim"];
    Ok(Topo {
        dim: f(&m["dim"], 4096.0) as usize,
        n_samples: f(&m["n_samples"], 1000.0) as usize,
        local_epochs: f(&m["local_epochs"], 3.0) as u32,
        nodes,
        weights,
        policy,
        train_slowdown: f(&s["train_slowdown"], 3.0),
        rto_min_ms: f(&s["rto_min_ms"], 200.0),
        jitter_ms: f(&s["jitter_ms"], 0.3),
        probe_timeout_ms: f(&v["scheduler"]["probe_timeout_ms"], 1500.0),
    })
}

fn binom(n: u32, p: f64, rng: &mut SplitMix64) -> u32 {
    (0..n).filter(|_| rng.next_f64() < p).count() as u32
}

/// One-way transfer time of `bytes`: propagation + serialisation + loss recovery.
fn transfer_ms(bytes: f64, delay_ms: f64, rate_mbit: f64, loss: f64, rtt_ms: f64, t: &Topo, rng: &mut SplitMix64) -> f64 {
    let jitter = (rng.next_normal() * t.jitter_ms).abs();
    let tx_ms = if rate_mbit.is_finite() && rate_mbit > 0.0 { bytes * 8.0 / (rate_mbit * 1e6) * 1000.0 } else { 0.0 };
    let segs = ((bytes / 1448.0).ceil() as u32).max(1);
    let mut lost = binom(segs, loss, rng);
    let mut rounds = 0u32;
    while lost > 0 && rounds < 10 {
        rounds += 1;
        lost = binom(lost, loss, rng); // retransmissions can be lost too
    }
    let rto = t.rto_min_ms.max(1.5 * rtt_ms);
    delay_ms + jitter + tx_ms + rounds as f64 * rto
}

fn base_record(cfg: &SimCfg, ty: &str, strategy: &str, rep: u32, inel: &[String]) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("type".into(), json!(ty));
    m.insert("label".into(), json!(cfg.label));
    m.insert("tier".into(), json!(cfg.tier));
    m.insert("substrate".into(), json!(cfg.substrate));
    m.insert("exp".into(), json!(cfg.exp));
    m.insert("cond_id".into(), json!(cfg.cond_id));
    m.insert("cond".into(), cfg.cond_json.clone());
    m.insert("strategy".into(), json!(strategy));
    m.insert("rep".into(), json!(rep));
    m.insert("ineligible".into(), json!(inel));
    m
}

fn simulate(cfg: &SimCfg, topo: &Topo) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let req_bytes = (4 * (topo.dim + 1) + 45) as f64;
    let resp_bytes = (4 * (topo.dim + 1) + 170) as f64;
    let probe_bytes = 100.0;
    let loss = cfg.loss_pct / 100.0;
    let cap_mbit = if cfg.rate_kbps > 0.0 { cfg.rate_kbps / 1000.0 } else { f64::INFINITY };
    let ops = topo.local_epochs as f64 * topo.n_samples as f64 * 2.0 * topo.dim as f64; // mul-adds

    // Per-invocation "measured" bench (real participants measure once at start-up).
    let mut brng = SplitMix64::new(derive_seed(cfg.base_seed, "bench-noise"));
    let bench: Vec<f64> = topo.nodes.iter().map(|n| n.class.bench_mops * (1.0 + 0.02 * brng.next_normal())).collect();

    let mut meta = base_record(cfg, "meta", "", 0, &cfg.ineligible_ids);
    meta.insert("git_sha".into(), json!(env::var("GIT_SHA").unwrap_or_else(|_| "unknown".into())));
    meta.insert("mode".into(), json!("sim"));
    meta.insert("strategies".into(), json!(cfg.strategies));
    meta.insert("reps".into(), json!(cfg.reps));
    meta.insert("rounds".into(), json!(cfg.rounds));
    meta.insert("warmup_rounds".into(), json!(cfg.warmup));
    meta.insert("k".into(), json!(cfg.k));
    meta.insert("min_participants".into(), json!(cfg.min_participants));
    meta.insert("deadline_ms".into(), json!(cfg.deadline_ms));
    meta.insert("base_seed".into(), json!(cfg.base_seed));
    meta.insert("down_ids".into(), json!(cfg.down_ids));
    meta.insert("loss_pct".into(), json!(cfg.loss_pct));
    meta.insert("delay_ms".into(), json!(cfg.delay_ms));
    meta.insert("rate_kbps".into(), json!(cfg.rate_kbps));
    out.push(Value::Object(meta));

    for rep in 0..cfg.reps {
        let seed = cfg.base_seed + rep as u64;
        let inel: Vec<String> = match cfg.ineligible_fraction {
            Some(fr) => {
                let mut ids: Vec<String> = topo.nodes.iter().map(|n| n.id.clone()).collect();
                SplitMix64::new(derive_seed(seed, "inel")).shuffle(&mut ids);
                ids.truncate((fr * ids.len() as f64).round() as usize);
                ids.sort();
                ids
            }
            None => cfg.ineligible_ids.clone(),
        };
        let is_inel: Vec<bool> = topo.nodes.iter().map(|n| inel.contains(&n.id)).collect();
        let is_down: Vec<bool> = topo.nodes.iter().map(|n| cfg.down_ids.contains(&n.id)).collect();

        let mut order = cfg.strategies.clone();
        SplitMix64::new(derive_seed(cfg.base_seed, &format!("order{rep}"))).shuffle(&mut order);

        for (oi, strat_s) in order.iter().enumerate() {
            let strategy = Strategy::parse(strat_s).unwrap_or_else(|| panic!("unknown strategy {strat_s}"));
            let run_id = format!("{}-{}-{}-rep{}", cfg.label, cfg.cond_id, strat_s, rep);
            let mut net = SplitMix64::new(derive_seed(seed, &format!("net:{strat_s}")));
            let mut avail: HashMap<String, f64> = HashMap::new();
            let mut hist = Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).expect("hist");
            let mut clock_ms = 0.0f64; // virtual clock

            for round in 0..cfg.rounds {
                // ---- probe phase (all nodes concurrently; random does not probe)
                let mut views: Vec<NodeView> = Vec::new();
                let mut select_ms = 0.0f64;
                for (i, n) in topo.nodes.iter().enumerate() {
                    let up_d = n.class.link_delay_ms + cfg.delay_ms;
                    let up_rate = n.class.link_rate_mbit.min(cap_mbit);
                    let mut reachable = true;
                    let mut rtt = 0.0;
                    if strategy != Strategy::Random {
                        if is_down[i] {
                            reachable = false; // connection refused / name not resolvable: fails fast
                        } else {
                            // The scheduler's own egress is NOT shaped in the testbed (keeps the
                            // OPA sidecar calls clean), so only the reply leg is impaired.
                            let r = up_d;
                            let p = transfer_ms(probe_bytes, 0.0, f64::INFINITY, 0.0, r, topo, &mut net)
                                + transfer_ms(probe_bytes, up_d, up_rate, loss, r, topo, &mut net);
                            if p > topo.probe_timeout_ms {
                                reachable = false;
                                select_ms = select_ms.max(topo.probe_timeout_ms);
                            } else {
                                rtt = p;
                                select_ms = select_ms.max(p);
                            }
                        }
                    }
                    views.push(NodeView {
                        id: n.id.clone(),
                        reachable,
                        jurisdiction: if is_inel[i] { "XX".into() } else { "UG".into() },
                        license_active: true,
                        residency_zone: if is_inel[i] { "xx-1".into() } else { "ug-central".into() },
                        bench_mops: bench[i],
                        bandwidth_mbps: n.class.link_rate_mbit,
                        rtt_ms: rtt,
                        availability: *avail.get(&n.id).unwrap_or(&1.0),
                    });
                }
                let verdicts: Vec<Result<(), String>> = views.iter().map(|v| topo.policy.check(v)).collect();
                let mut srng = SplitMix64::new(derive_seed(seed, &format!("{run_id}#{round}")));
                let sel = select(&views, &verdicts, cfg.k, strategy, &topo.weights, &mut srng);

                // ---- training phase
                let mut phase_ms = 0.0f64;
                let mut ok: Vec<usize> = Vec::new();
                let mut failed: Vec<(usize, &'static str)> = Vec::new();
                for (i, _score) in &sel.selected {
                    let n = &topo.nodes[*i];
                    if is_down[*i] {
                        failed.push((*i, "rpc_Unavailable"));
                        continue;
                    }
                    let up_d = n.class.link_delay_ms + cfg.delay_ms;
                    let down_d = cfg.delay_ms;
                    let r = up_d + down_d;
                    let up_rate = n.class.link_rate_mbit.min(cap_mbit);
                    let down = transfer_ms(req_bytes, down_d, cap_mbit, loss, r, topo, &mut net);
                    let compute = ops / (bench[*i] * 1e6) * 1000.0 * topo.train_slowdown * (1.0 + 0.03 * net.next_normal()).max(0.5);
                    let up = transfer_ms(resp_bytes, up_d, up_rate, loss, r, topo, &mut net);
                    let total = down + compute + up;
                    if total > cfg.deadline_ms {
                        failed.push((*i, "deadline"));
                        phase_ms = phase_ms.max(cfg.deadline_ms);
                    } else {
                        ok.push(*i);
                        phase_ms = phase_ms.max(total);
                    }
                }
                let round_ms = select_ms + phase_ms;
                let success = ok.len() >= cfg.min_participants;
                let warm = round < cfg.warmup;
                if !warm {
                    hist.record(((round_ms * 1000.0) as u64).max(1)).ok();
                }
                for i in &ok {
                    let a = avail.entry(topo.nodes[*i].id.clone()).or_insert(1.0);
                    *a = 0.7 * *a + 0.3;
                }
                for (i, _) in &failed {
                    let a = avail.entry(topo.nodes[*i].id.clone()).or_insert(1.0);
                    *a *= 0.7;
                }

                let mut rec = base_record(cfg, "round", strat_s, rep, &inel);
                rec.insert("order_index".into(), json!(oi));
                rec.insert("round".into(), json!(round));
                rec.insert("warmup".into(), json!(warm));
                rec.insert("t_unix_ms".into(), json!(clock_ms as u64));
                rec.insert(
                    "selected".into(),
                    json!(sel
                        .selected
                        .iter()
                        .map(|(i, s)| json!({"id": topo.nodes[*i].id, "rtt_ms": views[*i].rtt_ms, "score": s, "bench_mops": bench[*i]}))
                        .collect::<Vec<_>>()),
                );
                rec.insert(
                    "excluded".into(),
                    json!(sel.excluded.iter().map(|(i, r)| json!({"id": topo.nodes[*i].id, "reason": r})).collect::<Vec<_>>()),
                );
                rec.insert(
                    "responded".into(),
                    json!(ok
                        .iter()
                        .map(|i| json!({
                            "id": topo.nodes[*i].id,
                            "train_us": 0,
                            "loss": Value::Null,
                            "jurisdiction": views[*i].jurisdiction,
                            "license": "active",
                            "zone": views[*i].residency_zone,
                        }))
                        .collect::<Vec<_>>()),
                );
                rec.insert("failed".into(), json!(failed.iter().map(|(i, e)| json!({"id": topo.nodes[*i].id, "err": e})).collect::<Vec<_>>()));
                rec.insert("success".into(), json!(success));
                rec.insert("select_ms".into(), json!(select_ms));
                rec.insert("round_ms".into(), json!(round_ms));
                rec.insert("bytes_down".into(), json!((sel.selected.len() as f64 * req_bytes) as u64));
                rec.insert("bytes_up".into(), json!((ok.len() as f64 * resp_bytes) as u64));
                rec.insert("global_loss".into(), Value::Null);
                rec.insert("policy_disagreements".into(), json!(0));
                out.push(Value::Object(rec));
                clock_ms += round_ms;
            }

            let mut s = base_record(cfg, "run_summary", strat_s, rep, &inel);
            s.insert("n_rounds".into(), json!(hist.len()));
            s.insert("mean_ms".into(), json!(hist.mean() / 1000.0));
            s.insert("p50_ms".into(), json!(hist.value_at_quantile(0.50) as f64 / 1000.0));
            s.insert("p95_ms".into(), json!(hist.value_at_quantile(0.95) as f64 / 1000.0));
            s.insert("p99_ms".into(), json!(hist.value_at_quantile(0.99) as f64 / 1000.0));
            s.insert("max_ms".into(), json!(hist.max() as f64 / 1000.0));
            out.push(Value::Object(s));
        }
    }
    out
}

fn parse_args() -> HashMap<String, String> {
    let a: Vec<String> = env::args().skip(1).collect();
    let mut m = HashMap::new();
    let mut i = 0;
    while i < a.len() {
        if let Some(k) = a[i].strip_prefix("--") {
            if i + 1 < a.len() {
                m.insert(k.to_string(), a[i + 1].clone());
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    m
}

fn main() {
    let args = parse_args();
    let get = |k: &str, d: &str| args.get(k).cloned().unwrap_or_else(|| d.to_string());
    let getf = |k: &str, d: f64| args.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let list = |k: &str, d: &str| -> Vec<String> {
        get(k, d).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    let topo_path = get("topology", "config/topology.json");
    let topo_text = fs::read_to_string(&topo_path).unwrap_or_else(|e| panic!("cannot read {topo_path}: {e}"));
    let topo_json: Value = serde_json::from_str(&topo_text).expect("topology.json is not valid JSON");
    let topo = parse_topology(&topo_json).expect("invalid topology");

    let cond_json: Value = serde_json::from_str(&get("cond-json", "{}")).unwrap_or_else(|_| json!({}));
    let cfg = SimCfg {
        label: get("label", "sim"),
        tier: get("tier", "sim"),
        substrate: get("substrate", "flsim"),
        exp: get("exp", "adhoc"),
        cond_id: get("cond-id", "default"),
        cond_json,
        strategies: list("strategies", "random,resource_only,policy_resource"),
        reps: getf("reps", 30.0) as u32,
        rounds: getf("rounds", 20.0) as u32,
        warmup: getf("warmup", 2.0) as u32,
        k: getf("k", 4.0) as usize,
        min_participants: getf("min-participants", 3.0) as usize,
        deadline_ms: getf("deadline-ms", 5000.0),
        base_seed: getf("base-seed", 1000.0) as u64,
        ineligible_ids: list("ineligible", ""),
        ineligible_fraction: args.get("ineligible-fraction").and_then(|v| v.parse().ok()),
        down_ids: list("down", ""),
        loss_pct: getf("loss-pct", 0.0),
        delay_ms: getf("delay-ms", 0.0),
        rate_kbps: getf("rate-kbps", 0.0),
    };

    let out_dir = get("out-dir", "results/sim/raw");
    fs::create_dir_all(&out_dir).expect("cannot create out dir");
    let path = format!("{}/{}.jsonl", out_dir, cfg.label);
    let mut file = OpenOptions::new().create(true).append(true).open(&path).expect("cannot open output");
    for rec in simulate(&cfg, &topo) {
        writeln!(file, "{}", rec).expect("write failed");
    }
    eprintln!("flsim: wrote {} ({} reps x {} strategies)", path, cfg.reps, cfg.strategies.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topo() -> Topo {
        let v: Value = serde_json::from_str(
            r#"{
              "model": {"dim": 256, "n_samples": 100, "local_epochs": 2},
              "policy": {"allowed_jurisdictions": ["UG"], "allowed_zones": ["ug-central"]},
              "scheduler": {"probe_timeout_ms": 1500},
              "classes": {
                "fast": {"link_delay_ms": 5, "link_rate_mbit": 100, "sim_bench_mops": 2500},
                "slow": {"link_delay_ms": 60, "link_rate_mbit": 5, "sim_bench_mops": 600}
              },
              "nodes": [
                {"id":"a","class":"fast"},{"id":"b","class":"fast"},{"id":"c","class":"slow"},
                {"id":"d","class":"slow"},{"id":"e","class":"fast"},{"id":"f","class":"slow"}
              ]
            }"#,
        )
        .unwrap();
        parse_topology(&v).unwrap()
    }

    fn cfg(inel: &[&str]) -> SimCfg {
        SimCfg {
            label: "t".into(),
            tier: "sim".into(),
            substrate: "flsim".into(),
            exp: "T".into(),
            cond_id: "c".into(),
            cond_json: json!({}),
            strategies: vec!["random".into(), "resource_only".into(), "policy_resource".into()],
            reps: 5,
            rounds: 6,
            warmup: 1,
            k: 3,
            min_participants: 2,
            deadline_ms: 5000.0,
            base_seed: 11,
            ineligible_ids: inel.iter().map(|s| s.to_string()).collect(),
            ineligible_fraction: None,
            down_ids: vec![],
            loss_pct: 2.0,
            delay_ms: 20.0,
            rate_kbps: 0.0,
        }
    }

    #[test]
    fn identical_seed_gives_identical_output() {
        let t = topo();
        let c = cfg(&["a", "d"]);
        assert_eq!(simulate(&c, &t), simulate(&c, &t));
    }

    #[test]
    fn different_seed_gives_different_output() {
        let t = topo();
        let a = cfg(&["a", "d"]);
        let mut b = cfg(&["a", "d"]);
        b.base_seed = 12;
        assert_ne!(simulate(&a, &t), simulate(&b, &t));
    }

    #[test]
    fn policy_strategy_never_uses_ineligible_nodes() {
        let t = topo();
        let c = cfg(&["a", "d"]);
        for rec in simulate(&c, &t) {
            if rec["type"] == "round" && rec["strategy"] == "policy_resource" {
                for r in rec["responded"].as_array().unwrap() {
                    let id = r["id"].as_str().unwrap();
                    assert!(id != "a" && id != "d", "violation: {id}");
                }
            }
        }
    }

    #[test]
    fn resource_only_can_violate() {
        let t = topo();
        let c = cfg(&["a", "b", "e"]); // all the fast nodes are ineligible
        let mut violations = 0;
        for rec in simulate(&c, &t) {
            if rec["type"] == "round" && rec["strategy"] == "resource_only" {
                for r in rec["responded"].as_array().unwrap() {
                    if ["a", "b", "e"].contains(&r["id"].as_str().unwrap()) {
                        violations += 1;
                    }
                }
            }
        }
        assert!(violations > 0);
    }
}
