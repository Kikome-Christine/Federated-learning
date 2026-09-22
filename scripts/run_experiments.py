#!/usr/bin/env python3
"""Run the experiment matrix (config/experiments.json) on the sim or emu tier.

  python3 scripts/run_experiments.py --tier sim                # deterministic simulator (fast, no Docker)
  python3 scripts/run_experiments.py --tier emu                # docker compose + tc/netem (needs Docker)
  python3 scripts/run_experiments.py --tier sim --quick        # 3 reps x 5 rounds smoke test
  python3 scripts/run_experiments.py --tier emu --dry-run      # print every command, run nothing

The production tier (real hardware, real degraded network) is driven by hand
following docs/03_PRODUCTION_VALIDATION.md; it uses the same coordinator binary
and the same JSONL schema, so analysis is identical.
"""
import argparse, itertools, json, os, pathlib, platform, shlex, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXP = json.loads((ROOT / "config/experiments.json").read_text())
TOPO = json.loads((ROOT / "config/topology.json").read_text())
DC = ["docker", "compose", "--env-file", "docker/digests.env"]


def cond_id(exp, c):
    parts = [exp, c["pattern"], f"loss{c.get('loss_pct', 0)}", f"delay{c.get('delay_ms', 0)}",
             f"rate{c.get('rate_kbps', 'none')}"]
    if c.get("down"):
        parts.append("down" + "-".join(c["down"]))
    return "_".join(parts)


def all_conditions(only):
    out = []
    for e in EXP["experiments"]:
        if only and e["id"] not in only:
            continue
        for c in e["conditions"]:
            out.append((e, c))
    return out


def sh(cmd, dry, **kw):
    print("+", " ".join(shlex.quote(x) for x in cmd), flush=True)
    if not dry:
        subprocess.run(cmd, check=True, cwd=ROOT, **kw)


def params(args):
    d = dict(EXP["defaults"])
    if args.quick:
        d.update(reps=3, rounds=5)
    if args.reps: d["reps"] = args.reps
    if args.rounds: d["rounds"] = args.rounds
    return d


# ---------------------------------------------------------------- sim tier
def run_sim(args):
    d = params(args)
    out = pathlib.Path(args.results_root).resolve() / "sim/raw" if args.results_root else ROOT / "results/sim/raw"
    out.mkdir(parents=True, exist_ok=True)
    binary = os.environ.get("FLSIM", "target/release/flsim")
    for e, c in all_conditions(args.exp):
        if "sim" not in e["tiers"]:
            continue
        cid = cond_id(e["id"], c)
        cmd = [binary, "--topology", "config/topology.json", "--out-dir", str(out),
               "--label", cid, "--exp", e["id"], "--cond-id", cid, "--cond-json", json.dumps(c),
               "--strategies", ",".join(d["strategies"]), "--reps", str(d["reps"]), "--rounds", str(d["rounds"]),
               "--warmup", str(d["warmup_rounds"]), "--k", str(d["k"]), "--min-participants", str(d["min_participants"]),
               "--deadline-ms", str(d["deadline_ms"]), "--base-seed", str(d["base_seed"]),
               "--ineligible", ",".join(TOPO["ineligible_patterns"][c["pattern"]]),
               "--loss-pct", str(c.get("loss_pct", 0)), "--delay-ms", str(c.get("delay_ms", 0)),
               "--rate-kbps", str(c.get("rate_kbps", 0)), "--down", ",".join(c.get("down", []))]
        if not args.dry_run:
            (out / f"{cid}.jsonl").unlink(missing_ok=True)  # re-runs must not append duplicates
        sh(cmd, args.dry_run)
    if not args.dry_run:
        write_env(out.parent / "env.json", args)


# ---------------------------------------------------------------- emu tier
def netem_args(node, c):
    cls = TOPO["classes"][node["class"]]
    delay = cls["link_delay_ms"] + c.get("delay_ms", 0)
    rate_kbit = int(cls["link_rate_mbit"] * 1000)
    if c.get("rate_kbps"):
        rate_kbit = min(rate_kbit, int(c["rate_kbps"]))
    a = [f"delay {delay}ms"] if delay else []
    if c.get("loss_pct"):
        a.append(f"loss {c['loss_pct']}%")
    a.append(f"rate {rate_kbit}kbit")
    return " ".join(a)


def coordinator_netem(c):
    a = []
    if c.get("delay_ms"): a.append(f"delay {c['delay_ms']}ms")
    if c.get("loss_pct"): a.append(f"loss {c['loss_pct']}%")
    if c.get("rate_kbps"): a.append(f"rate {int(c['rate_kbps'])}kbit")
    return " ".join(a)


def git_sha():
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()
    except Exception:
        return "unknown"


def run_emu(args):
    d = params(args)
    ids = [n["id"] for n in TOPO["nodes"]]
    sha = git_sha()
    sh(DC + ["build", "scheduler"], args.dry_run)
    current_pattern = None
    for e, c in all_conditions(args.exp):
        if "emu" not in e["tiers"]:
            continue
        cid = cond_id(e["id"], c)
        inel = ",".join(TOPO["ineligible_patterns"][c["pattern"]])
        env = dict(os.environ, INELIGIBLE_IDS=inel, GIT_SHA=sha)
        print(f"\n=== {cid}", flush=True)
        if c["pattern"] != current_pattern:
            sh(DC + ["up", "-d", "--force-recreate", "opa", "scheduler"] + [f"participant-{i}" for i in ids],
               args.dry_run, env=env)
            current_pattern = c["pattern"]
            if not args.dry_run: time.sleep(8)
        down = c.get("down", [])
        sh(DC + ["start"] + [f"participant-{i}" for i in ids if i not in down], args.dry_run, env=env)
        if down:
            sh(DC + ["stop"] + [f"participant-{i}" for i in down], args.dry_run, env=env)
        for n in TOPO["nodes"]:
            if n["id"] in down: continue
            sh(DC + ["exec", "-T", f"participant-{n['id']}", "sh", "-c",
                     f"tc qdisc replace dev eth0 root netem {netem_args(n, c)}"], args.dry_run, env=env)
        if not args.dry_run: time.sleep(3)
        label = cid
        if not args.dry_run:
            (ROOT / f"results/emu/raw/{label}.jsonl").unlink(missing_ok=True)
        run_env = {"TIER": "emu", "SUBSTRATE": "compose", "EXP": e["id"], "COND_ID": cid, "COND_JSON": json.dumps(c),
                   "RUN_LABEL": label, "REPS": d["reps"], "ROUNDS": d["rounds"], "WARMUP_ROUNDS": d["warmup_rounds"],
                   "K": d["k"], "MIN_PARTICIPANTS": d["min_participants"], "DEADLINE_MS": d["deadline_ms"],
                   "BASE_SEED": d["base_seed"], "STRATEGIES": ",".join(d["strategies"]), "INELIGIBLE_IDS": inel,
                   "OUT_DIR": "/results/emu/raw", "NETEM_ARGS": coordinator_netem(c), "GIT_SHA": sha}
        cmd = DC + ["--profile", "run", "run", "--rm", "-T"]
        for k, v in run_env.items():
            cmd += ["-e", f"{k}={v}"]
        cmd += ["coordinator", "sh", "-c",
                'if [ -n "$NETEM_ARGS" ]; then tc qdisc replace dev eth0 root netem $NETEM_ARGS; fi; exec coordinator']
        sh(cmd, args.dry_run, env=env)
        for n in TOPO["nodes"]:
            if n["id"] in down: continue
            sh(DC + ["exec", "-T", f"participant-{n['id']}", "tc", "qdisc", "del", "dev", "eth0", "root"], args.dry_run, env=env)

    # Mandatory naive baseline (centralized single node), pattern from experiments.json
    pat = EXP["centralized_baseline"]["pattern"]
    inel = ",".join(TOPO["ineligible_patterns"][pat])
    label = f"E0_centralized_{pat}"
    if not args.dry_run:
        (ROOT / f"results/emu/raw/{label}.jsonl").unlink(missing_ok=True)
    cmd = DC + ["--profile", "run", "run", "--rm", "-T"]
    for k, v in {"MODE": "centralized", "TIER": "emu", "SUBSTRATE": "centralized", "EXP": "E0", "COND_ID": label,
                 "RUN_LABEL": label, "REPS": d["reps"], "ROUNDS": d["rounds"], "WARMUP_ROUNDS": d["warmup_rounds"],
                 "INELIGIBLE_IDS": inel, "OUT_DIR": "/results/emu/raw", "BASE_SEED": d["base_seed"], "GIT_SHA": sha}.items():
        cmd += ["-e", f"{k}={v}"]
    sh(cmd + ["coordinator"], args.dry_run)
    sh(DC + ["down"], args.dry_run)
    if not args.dry_run:
        write_env(ROOT / "results/emu/env.json", args)


def write_env(path, args):
    info = {"git_sha": git_sha(), "platform": platform.platform(), "python": platform.python_version(),
            "machine": platform.machine(), "cpu_count": os.cpu_count(), "argv": sys.argv,
            "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    for name, cmd in {"docker": ["docker", "--version"], "rustc": ["rustc", "--version"]}.items():
        try:
            info[name] = subprocess.check_output(cmd, text=True).strip()
        except Exception:
            info[name] = None
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(info, indent=2))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tier", required=True, choices=["sim", "emu"])
    ap.add_argument("--exp", type=lambda s: set(s.split(",")), default=None, help="e.g. E1,E2")
    ap.add_argument("--reps", type=int); ap.add_argument("--rounds", type=int)
    ap.add_argument("--results-root", help="sim tier only: write raw data here instead of ./results (used by `make smoke`)")
    ap.add_argument("--quick", action="store_true"); ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    (ROOT / f"results/{a.tier}/raw").mkdir(parents=True, exist_ok=True)
    (run_sim if a.tier == "sim" else run_emu)(a)


if __name__ == "__main__":
    main()
