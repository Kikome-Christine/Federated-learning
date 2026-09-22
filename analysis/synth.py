"""SYNTHETIC data generator -- used ONLY by `analyze.py --self-test` and the tests
to exercise the pipeline. Everything it writes is labelled SYNTHETIC and must
never be mixed with real results or quoted in the paper."""
import json, pathlib
import numpy as np

NODES = [f"n{i}" for i in range(1, 9)]


def write_synthetic(root: pathlib.Path, tier="sim", loss_levels=(0, 5), reps=30, rounds=10, seed=1,
                    shift=1.0):
    rng = np.random.default_rng(seed)
    raw = root / tier / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    inel = ["n2", "n6"]
    conds = [("E1", {"pattern": "f0.25"}, "E1_f0.25_loss0_delay0_ratenone", 0)]
    for L in loss_levels:
        if L:
            conds.append(("E2", {"pattern": "f0.25", "loss_pct": L}, f"E2_f0.25_loss{L}_delay0_ratenone", L))
    for exp, cond, cid, L in conds:
        with open(raw / f"{cid}.jsonl", "w") as fh:
            for rep in range(reps):
                for strat, base in (("random", 400), ("resource_only", 250), ("policy_resource", 280)):
                    for rd in range(rounds):
                        lat = float(rng.lognormal(np.log(base * shift * (1 + 0.15 * L)), 0.25))
                        pool = NODES if strat != "policy_resource" else [n for n in NODES if n not in inel]
                        sel = list(rng.choice(pool, size=4, replace=False))
                        rec = {"type": "round", "label": "SYNTHETIC", "tier": tier, "substrate": "synthetic",
                               "exp": exp, "cond_id": cid, "cond": cond, "strategy": strat, "rep": rep,
                               "round": rd, "warmup": rd < 2, "ineligible": inel, "selected": [{"id": s} for s in sel],
                               "responded": [{"id": s, "jurisdiction": "XX" if s in inel else "UG", "license": "active"} for s in sel],
                               "success": True, "select_ms": 5.0, "round_ms": lat, "bytes_down": 16000 * 4,
                               "bytes_up": 16000 * 4, "global_loss": None, "policy_disagreements": 0}
                        fh.write(json.dumps(rec) + "\n")
    return raw
