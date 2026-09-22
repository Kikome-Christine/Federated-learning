#!/usr/bin/env python3
"""Regenerate every number, table and figure in the manuscript from raw JSONL.

    python3 analysis/analyze.py --results results --topology config/topology.json \
            --out results/tables --paper-out paper/generated
    python3 analysis/analyze.py --self-test        # pipeline check on SYNTHETIC data

Reads results/<tier>/raw/*.jsonl (tier = sim | emu | prod), one JSON object per
round (schema: crates/coordinator/src/main.rs). Nothing is filtered except the
declared warm-up rounds; duplicate rows are reported, never silently dropped.
"""
from __future__ import annotations

import argparse, json, math, pathlib, sys, tempfile
from collections import defaultdict

import numpy as np
import pandas as pd

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import stats as S  # noqa: E402

KEYS = ["tier", "substrate", "exp", "cond_id", "strategy"]
STRATS = ["random", "resource_only", "policy_resource"]
SHORT = {"random": "Random", "resource_only": "Resource-only", "policy_resource": "Policy+Resource",
         "centralized": "Centralized"}
TOL = 0.20  # pre-declared sim-vs-production tolerance (exam Sec. 5.1)


# --------------------------------------------------------------------- load
def load(results: pathlib.Path, allowed_jur=("UG",)):
    rows, bad, dup = [], 0, 0
    seen = {}
    for p in sorted(results.rglob("*.jsonl")):
        with open(p) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except json.JSONDecodeError:
                    bad += 1
                    continue
                if r.get("type") != "round":
                    continue
                inel = set(r.get("ineligible", []))
                resp = r.get("responded", [])
                ids = [x["id"] for x in resp]
                att = sum(1 for x in resp if "jurisdiction" in x and
                          (x["jurisdiction"] not in allowed_jur or x.get("license", "active") != "active"))
                row = dict(
                    tier=r["tier"], substrate=r.get("substrate", ""), exp=r.get("exp", ""), cond_id=r["cond_id"],
                    strategy=r["strategy"], rep=int(r["rep"]), round=int(r["round"]), warmup=bool(r.get("warmup")),
                    success=bool(r.get("success")), select_ms=r.get("select_ms"), round_ms=float(r["round_ms"]),
                    bytes=(r.get("bytes_down") or 0) + (r.get("bytes_up") or 0),
                    global_loss=r.get("global_loss"), viol=sum(1 for i in ids if i in inel), attest_viol=att,
                    n_resp=len(ids), selected=tuple(x["id"] for x in r.get("selected", [])),
                    ineligible=tuple(sorted(inel)), cond=r.get("cond", {}), label=r.get("label", ""),
                    disagree=r.get("policy_disagreements") or 0,
                )
                k = (row["tier"], row["substrate"], row["cond_id"], row["strategy"], row["rep"], row["round"])
                if k in seen:
                    dup += 1
                seen[k] = row  # last write wins, but we report it
    rows = list(seen.values())
    if bad:
        print(f"WARNING: {bad} unparsable lines skipped", file=sys.stderr)
    if dup:
        print(f"WARNING: {dup} duplicate (tier,substrate,cond,strategy,rep,round) rows - last kept. "
              f"Delete stale raw files before re-running.", file=sys.stderr)
    df = pd.DataFrame(rows)
    if len(df) and df["label"].str.startswith("SYNTHETIC").any():
        print("*** SYNTHETIC DATA DETECTED - pipeline test only, NOT results ***", file=sys.stderr)
    return df, dup


def pretty(cond: dict) -> str:
    p = cond.get("pattern", "")
    parts = [f"f={p[1:]}" if p.startswith("f") else p or "-"]
    if cond.get("loss_pct"): parts.append(f"loss {cond['loss_pct']}%")
    if cond.get("delay_ms"): parts.append(f"+{cond['delay_ms']} ms")
    if cond.get("rate_kbps"): parts.append(f"cap {cond['rate_kbps']} kbit/s")
    if cond.get("down"): parts.append(f"down {len(cond['down'])}")
    if cond.get("real"): parts.append("REAL link")
    return ", ".join(parts)


# ---------------------------------------------------------------- rep level
def rep_level(df: pd.DataFrame, nodes: list[str]) -> pd.DataFrame:
    out = []
    for key, g in df.groupby(KEYS + ["rep"]):
        g = g.sort_values("round")
        inel = set(g["ineligible"].iloc[0])
        elig = [n for n in nodes if n not in inel]
        counts = defaultdict(int)
        for sel in g["selected"]:
            for i in sel:
                counts[i] += 1
        losses = g["global_loss"].dropna()
        out.append(dict(zip(KEYS + ["rep"], key)) | dict(
            n_rounds=len(g), success_rate=float(g["success"].mean()), viol_total=int(g["viol"].sum()),
            attest_viol_total=int(g["attest_viol"].sum()), viol_per_round=float(g["viol"].mean()),
            mean_round_ms=float(g["round_ms"].mean()), mean_select_ms=float(pd.to_numeric(g["select_ms"]).mean()),
            bytes_per_round=float(g["bytes"].mean()), final_loss=float(losses.iloc[-1]) if len(losses) else math.nan,
            jain=S.jain([counts[n] for n in elig]) if elig else math.nan, disagree=int(g["disagree"].sum()),
            cond=g["cond"].iloc[0]))
    return pd.DataFrame(out)


# ------------------------------------------------------------------ summary
def summarize(df, rl, n_boot, seed):
    rows = []
    for key, g in df.groupby(KEYS):
        reps = [v.to_numpy() for _, v in g.groupby("rep")["round_ms"]]
        pc = S.cluster_boot_percentiles(reps, (50, 95, 99), n_boot, seed)
        r = rl[(rl[KEYS] == pd.Series(key, index=KEYS)).all(axis=1)]
        m_lat = S.t_ci(r["mean_round_ms"]); m_suc = S.t_ci(r["success_rate"]); m_vpr = S.t_ci(r["viol_per_round"])
        m_byt = S.t_ci(r["bytes_per_round"]); m_jn = S.t_ci(r["jain"]); m_los = S.t_ci(r["final_loss"])
        row = dict(zip(KEYS, key)) | dict(n_reps=len(reps), n_rounds=int(len(g)), cond=g["cond"].iloc[0])
        for q in (50, 95, 99):
            row[f"p{q}"], row[f"p{q}_lo"], row[f"p{q}_hi"] = pc[q]
        for name, t in (("mean_ms", m_lat), ("success", m_suc), ("viol_per_round", m_vpr), ("bytes", m_byt),
                        ("jain", m_jn), ("final_loss", m_los)):
            row[name], row[name + "_lo"], row[name + "_hi"] = t
        row["viol_total"] = int(r["viol_total"].sum()); row["attest_viol_total"] = int(r["attest_viol_total"].sum())
        row["policy_disagreements"] = int(r["disagree"].sum())
        rows.append(row)
    return pd.DataFrame(rows)


def rep_arrays(df, tier, substrate, cond_id, strategy):
    g = df[(df.tier == tier) & (df.substrate == substrate) & (df.cond_id == cond_id) & (df.strategy == strategy)]
    return {int(k): v["round_ms"].to_numpy() for k, v in g.groupby("rep")}


def comparisons(df, rl, n_boot, seed):
    rows = []
    for (tier, sub, exp, cid), g in df.groupby(["tier", "substrate", "exp", "cond_id"]):
        if "policy_resource" not in set(g.strategy):
            continue
        cond = g["cond"].iloc[0]
        pr = rep_arrays(df, tier, sub, cid, "policy_resource")
        for other in ("resource_only", "random"):
            if other not in set(g.strategy):
                continue
            ot = rep_arrays(df, tier, sub, cid, other)
            row = dict(tier=tier, substrate=sub, exp=exp, cond_id=cid, cond=cond, vs=other)
            for q in (50, 95, 99):
                est, lo, hi, n = S.paired_ratio_ci(pr, ot, q, n_boot, seed)
                row[f"ratio_p{q}"], row[f"ratio_p{q}_lo"], row[f"ratio_p{q}_hi"], row["n_paired"] = est, lo, hi, n
            a = rl[(rl.tier == tier) & (rl.substrate == sub) & (rl.cond_id == cid) & (rl.strategy == "policy_resource")]["mean_round_ms"]
            b = rl[(rl.tier == tier) & (rl.substrate == sub) & (rl.cond_id == cid) & (rl.strategy == other)]["mean_round_ms"]
            row["cohens_d_mean"] = S.cohens_d(a, b); row["cliffs_delta_mean"] = S.cliffs_delta(a, b)
            row["cliffs_label"] = S.cliffs_label(row["cliffs_delta_mean"])
            rows.append(row)
    return pd.DataFrame(rows)


# --------------------------------------------------------------- hypotheses
def hypotheses(summ, comp):
    """Verdicts follow the decision rules in docs/01_PREREGISTRATION.md, mechanically."""
    out = []
    for tier in sorted(summ.tier.unique()):
        s = summ[(summ.tier == tier) & (summ.substrate != "centralized")]
        pr = s[s.strategy == "policy_resource"]
        # H1 -- compliance: zero violations for policy_resource, everywhere.
        v = int(pr["viol_total"].sum()); va = int(pr["attest_viol_total"].sum())
        out.append(dict(tier=tier, hyp="H1", scope="all conditions",
                        statement="policy_resource admits 0 ineligible participants",
                        result=f"{v} violations (ground truth), {va} (attestation audit) over {int(pr.n_rounds.sum())} rounds",
                        verdict="supported" if v == 0 and va == 0 else "FALSIFIED"))
        # manipulation check
        for st in ("resource_only", "random"):
            o = s[(s.strategy == st) & (s.cond.apply(lambda c: c.get("pattern") not in (None, "f0.00")))]
            out.append(dict(tier=tier, hyp="H1-check", scope=st, statement="policy-blind baseline DOES violate when f>0",
                            result=f"{int(o['viol_total'].sum())} violations", verdict="ok" if o["viol_total"].sum() > 0 else "CHECK DESIGN"))
        c = comp[(comp.tier == tier) & (comp.substrate != "centralized")]
        # H2 -- cost of compliance
        for _, r in c[(c.vs == "resource_only") & (c.exp == "E1")].iterrows():
            pat = r["cond"].get("pattern")
            d = S.decide_le(r.ratio_p99_lo, r.ratio_p99_hi, 1.20)
            if pat == "f0.50":   # pre-registered PREDICTED break point: overhead exceeds 20 %
                hyp_id, stmt = "H2b", "p99(policy)/p99(resource_only) > 1.20 (predicted break point)"
                d = {"falsified": "supported", "supported": "falsified"}.get(d, d)
            else:
                hyp_id, stmt = "H2", "p99(policy)/p99(resource_only) <= 1.20"
            out.append(dict(tier=tier, hyp=hyp_id, scope=pretty(r["cond"]), statement=stmt,
                            result=f"{r.ratio_p99:.2f} [{r.ratio_p99_lo:.2f}, {r.ratio_p99_hi:.2f}]", verdict=d))
        # H3 -- resource-awareness pays off under degradation
        for _, r in c[(c.vs == "random") & (c.exp == "E2") & (c.cond.apply(lambda x: x.get("loss_pct", 0) >= 5))].iterrows():
            out.append(dict(tier=tier, hyp="H3", scope=pretty(r["cond"]), statement="p99(policy)/p99(random) < 1",
                            result=f"{r.ratio_p99:.2f} [{r.ratio_p99_lo:.2f}, {r.ratio_p99_hi:.2f}]",
                            verdict=S.decide_lt(r.ratio_p99_lo, r.ratio_p99_hi)))
    return pd.DataFrame(out)


# ------------------------------------------------------------ sim vs prod
def sim_vs_prod(comp, summ, pairs):
    """Headline quantities side by side; 'reproduces' iff same direction AND |rel diff| <= 20 %."""
    def get(tier, cid):
        c = comp[(comp.tier == tier) & (comp.cond_id == cid)]
        s = summ[(summ.tier == tier) & (summ.cond_id == cid) & (summ.strategy == "policy_resource")]
        d = {}
        if len(s): d["p99_policy"] = float(s.p99.iloc[0])
        for vs in ("resource_only", "random"):
            r = c[c.vs == vs]
            if len(r): d[f"p99_ratio_vs_{vs}"] = float(r.ratio_p99.iloc[0])
        return d
    rows = []
    tiers = sorted(summ.tier.unique())
    todo = [(a, ca, b, cb) for (a, ca, b, cb) in pairs]
    if not pairs:
        for cid in sorted(summ.cond_id.unique()):
            have = [t for t in tiers if len(summ[(summ.tier == t) & (summ.cond_id == cid)])]
            for i in range(len(have)):
                for j in range(i + 1, len(have)):
                    todo.append((have[i], cid, have[j], cid))
    for ta, ca, tb, cb in todo:
        A, B = get(ta, ca), get(tb, cb)
        for m in sorted(set(A) & set(B)):
            rel = (B[m] - A[m]) / A[m] if A[m] else math.nan
            dir_a = (A[m] > 1) if "ratio" in m else None
            same_dir = True if dir_a is None else ((A[m] > 1) == (B[m] > 1))
            rows.append(dict(cond_a=f"{ta}:{ca}", cond_b=f"{tb}:{cb}", metric=m, value_a=A[m], value_b=B[m],
                             rel_diff=rel, same_direction=same_dir,
                             reproduces=bool(same_dir and abs(rel) <= TOL)))
    return pd.DataFrame(rows)


# ---------------------------------------------------------------- trace
def trace_summary(results: pathlib.Path):
    rows = []
    for p in sorted(results.rglob("trace*.csv")):
        t = pd.read_csv(p)
        ping = t[t.kind == "ping"]
        if not len(ping):
            continue
        rtt = pd.to_numeric(ping.rtt_ms, errors="coerce").dropna()
        ip = pd.to_numeric(t[t.kind == "iperf"].mbit_s, errors="coerce").dropna()
        rows.append(dict(trace=p.name, duration_s=(ping.t_unix_ms.max() - ping.t_unix_ms.min()) / 1000, n=len(ping),
                         loss_pct=100 * float(ping.lost.mean()), rtt_p50=float(rtt.median()) if len(rtt) else math.nan,
                         rtt_p95=float(rtt.quantile(.95)) if len(rtt) else math.nan,
                         rtt_max=float(rtt.max()) if len(rtt) else math.nan,
                         mbit_median=float(ip.median()) if len(ip) else math.nan))
    return pd.DataFrame(rows)


# ------------------------------------------------------------------ LaTeX
def esc(s) -> str:
    return str(s).replace("\\", "").replace("_", r"\_").replace("%", r"\%").replace("&", r"\&").replace("#", r"\#")


def ci(est, lo, hi, nd=0):
    if est is None or (isinstance(est, float) and math.isnan(est)):
        return "--"
    if lo is None or math.isnan(lo):
        return f"{est:.{nd}f}"
    return f"{est:.{nd}f} [{lo:.{nd}f}, {hi:.{nd}f}]"


def tabular(header, rows, align=None):
    align = align or "l" * len(header)
    L = [r"\begin{tabular}{" + align + "}", r"\toprule", " & ".join(header) + r" \\", r"\midrule"]
    L += [" & ".join(r) + r" \\" for r in rows]
    L += [r"\bottomrule", r"\end{tabular}"]
    return "\n".join(L) + "\n"


def write_tex(outdir: pathlib.Path, summ, comp, hyp, svp, tr, dfraw):
    outdir.mkdir(parents=True, exist_ok=True)
    macros = [r"% GENERATED by analysis/analyze.py -- do not edit"]
    for tier in sorted(summ.tier.unique()):
        s = summ[(summ.tier == tier) & (summ.substrate != "centralized")]
        for exp in sorted(s.exp.unique()):
            e = s[s.exp == exp].sort_values(["cond_id", "strategy"])
            rows = []
            for _, r in e.iterrows():
                rows.append([esc(pretty(r.cond)), esc(SHORT.get(r.strategy, r.strategy)), ci(r.p50, r.p50_lo, r.p50_hi),
                             ci(r.p95, r.p95_lo, r.p95_hi), ci(r.p99, r.p99_lo, r.p99_hi),
                             f"{r.success:.2f}", str(int(r.viol_total)), f"{r.bytes / 1024:.1f}"])
            (outdir / f"tab_{exp.lower()}_{tier}.tex").write_text(tabular(
                ["Condition", "Strategy", "p50 (ms)", "p95 (ms)", "p99 (ms)", "Succ.", "Viol.", "KiB/rd"], rows, "llrrrrrr"))
        c = comp[(comp.tier == tier) & (comp.substrate != "centralized")].sort_values(["exp", "cond_id", "vs"])
        rows = [[esc(pretty(r["cond"])), esc(SHORT[r.vs]), ci(r.ratio_p50, r.ratio_p50_lo, r.ratio_p50_hi, 2),
                 ci(r.ratio_p99, r.ratio_p99_lo, r.ratio_p99_hi, 2), f"{r.cohens_d_mean:.2f}",
                 f"{r.cliffs_delta_mean:.2f} ({r.cliffs_label})"] for _, r in c.iterrows()]
        if rows:
            (outdir / f"tab_ratios_{tier}.tex").write_text(tabular(
                ["Condition", "Policy+Resource vs", "p50 ratio", "p99 ratio", "Cohen $d$", "Cliff $\\delta$"], rows, "llrrrr"))
        h = hyp[hyp.tier == tier]
        if len(h):
            (outdir / f"tab_hypotheses_{tier}.tex").write_text(tabular(
                ["Hyp.", "Scope", "Result", "Verdict"], [[r.hyp, esc(r.scope), esc(r.result), esc(r.verdict)] for _, r in h.iterrows()], "llll"))
        # headline macros (only when the value exists)
        b = c[(c.vs == "resource_only") & (c.cond.apply(lambda x: x.get("pattern") == "f0.25" and not any(x.get(k) for k in ("loss_pct", "delay_ms", "rate_kbps", "down"))))]
        if len(b):
            macros.append(rf"\newcommand{{\HTwoRatio{tier.capitalize()}}}{{{b.ratio_p99.iloc[0]:.2f}}}")
            macros.append(rf"\newcommand{{\HTwoRatioCI{tier.capitalize()}}}{{[{b.ratio_p99_lo.iloc[0]:.2f}, {b.ratio_p99_hi.iloc[0]:.2f}]}}")
    cen = summ[summ.strategy == "centralized"]
    if len(cen):
        (outdir / "tab_centralized.tex").write_text(tabular(
            ["Tier", "Mean round (ms) [95\\% CI]", "Ineligible data pooled (count)"],
            [[esc(r.tier), ci(r.mean_ms, r.mean_ms_lo, r.mean_ms_hi, 1), str(int(r.viol_total))] for _, r in cen.iterrows()], "lrr"))
    subs = summ[(summ.strategy.isin(["policy_resource", "centralized"])) & (summ.cond_id.str.contains("f0.25_loss0_delay0_ratenone|E0_centralized"))]
    if subs.substrate.nunique() > 1:
        (outdir / "tab_substrates.tex").write_text(tabular(
            ["Tier", "Substrate", "p50 (ms)", "p99 (ms)", "Viol."],
            [[esc(r.tier), esc(r.substrate), ci(r.p50, r.p50_lo, r.p50_hi), ci(r.p99, r.p99_lo, r.p99_hi), str(int(r.viol_total))]
             for _, r in subs.sort_values(["tier", "substrate"]).iterrows()], "llrrr"))
    if len(svp):
        lab = {f"{r.tier}:{r.cond_id}": f"{r.tier}: {pretty(r.cond)}" for _, r in summ.drop_duplicates(["tier", "cond_id"]).iterrows()}
        (outdir / "tab_sim_vs_prod.tex").write_text(tabular(
            ["Condition A", "Condition B", "Metric", "A", "B", "Rel. diff", "Reproduces ($\\pm$20\\%)"],
            [[esc(lab.get(r.cond_a, r.cond_a)), esc(lab.get(r.cond_b, r.cond_b)), esc(r.metric.replace("p99_ratio_vs_", "p99 ratio vs ")), f"{r.value_a:.2f}", f"{r.value_b:.2f}", f"{100 * r.rel_diff:+.1f}\\%",
              "yes" if r.reproduces else "\\textbf{no}"] for _, r in svp.iterrows()], "lllrrrl"))
    if len(tr):
        (outdir / "tab_trace.tex").write_text(tabular(
            ["Trace", "Duration (s)", "Loss (\\%)", "RTT p50", "RTT p95", "RTT max (ms)", "Mbit/s"],
            [[esc(r.trace), f"{r.duration_s:.0f}", f"{r.loss_pct:.1f}", f"{r.rtt_p50:.0f}", f"{r.rtt_p95:.0f}", f"{r.rtt_max:.0f}",
              "--" if math.isnan(r.mbit_median) else f"{r.mbit_median:.2f}"] for _, r in tr.iterrows()], "lrrrrrr"))
    macros.append(rf"\newcommand{{\NumReps}}{{{int(summ.n_reps.max()) if len(summ) else 0}}}")
    (outdir / "macros.tex").write_text("\n".join(macros) + "\n")


# ---------------------------------------------------------------- figures
def figures(outdir: pathlib.Path, summ):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    outdir.mkdir(parents=True, exist_ok=True)
    colors = {"random": "#999999", "resource_only": "#d95f02", "policy_resource": "#1b9e77"}
    for tier in sorted(summ.tier.unique()):
        s = summ[(summ.tier == tier) & (summ.substrate != "centralized")]
        panels = [("loss_pct", "packet loss (%)"), ("delay_ms", "added one-way delay (ms)"), ("rate_kbps", "bandwidth cap (kbit/s)")]
        active = []
        base = s[s.cond.apply(lambda c: c.get("pattern") == "f0.25" and not any(c.get(k) for k in ("loss_pct", "delay_ms", "rate_kbps", "down")))]
        for key, label in panels:
            sub = s[s.cond.apply(lambda c, k=key: bool(c.get(k)) and c.get("pattern") == "f0.25" and not c.get("down")
                                 and all(not c.get(o) for o in ("loss_pct", "delay_ms", "rate_kbps") if o != k))]
            if len(sub):
                active.append((key, label, sub))
        if active:
            fig, axes = plt.subplots(1, len(active), figsize=(4.2 * len(active), 3.2), squeeze=False)
            for ax, (key, label, sub) in zip(axes[0], active):
                for st in STRATS:
                    pts = pd.concat([base[base.strategy == st].assign(x=0.0 if key != "rate_kbps" else np.nan),
                                     sub[sub.strategy == st].assign(x=lambda d, k=key: d.cond.apply(lambda c: float(c[k])))]).dropna(subset=["x"]).sort_values("x")
                    if not len(pts): continue
                    ax.errorbar(pts.x, pts.p99, yerr=[pts.p99 - pts.p99_lo, pts.p99_hi - pts.p99], marker="o", capsize=3,
                                color=colors[st], label=SHORT[st])
                ax.set_xlabel(label); ax.set_ylabel("p99 round latency (ms)"); ax.grid(alpha=.3)
            axes[0][0].legend(fontsize=7)
            fig.tight_layout(); fig.savefig(outdir / f"fig_e2_p99_{tier}.pdf"); fig.savefig(outdir / f"fig_e2_p99_{tier}.png", dpi=160); plt.close(fig)
        e1 = s[s.exp == "E1"]
        if len(e1):
            pats = sorted(e1.cond.apply(lambda c: c.get("pattern")).unique())
            fig, ax = plt.subplots(figsize=(4.2, 3.2)); w = 0.25
            for i, st in enumerate(STRATS):
                vals = [e1[(e1.strategy == st) & (e1.cond.apply(lambda c: c.get("pattern")) == p)]["viol_per_round"].mean() for p in pats]
                ax.bar(np.arange(len(pats)) + i * w, vals, w, color=colors[st], label=SHORT[st])
            ax.set_xticks(np.arange(len(pats)) + w); ax.set_xticklabels([f"f={p[1:]}" for p in pats])
            ax.set_ylabel("violations per round"); ax.legend(fontsize=7); ax.grid(alpha=.3, axis="y")
            fig.tight_layout(); fig.savefig(outdir / f"fig_e1_violations_{tier}.pdf"); fig.savefig(outdir / f"fig_e1_violations_{tier}.png", dpi=160); plt.close(fig)


def md_report(outdir, summ, comp, hyp, svp, dup):
    L = ["# Analysis report (generated)\n"]
    if dup: L.append(f"> WARNING: {dup} duplicate rows detected in raw data.\n")
    L.append("## Hypothesis verdicts (mechanical, per pre-registered decision rules)\n")
    L.append(hyp.to_markdown(index=False) if len(hyp) else "_no data_")
    L.append("\n## Sim-vs-production / cross-tier comparison (tolerance +-20 %)\n")
    L.append(svp.to_markdown(index=False) if len(svp) else "_only one tier present_")
    (outdir / "REPORT.md").write_text("\n".join(L) + "\n")


# ------------------------------------------------------------------- main
def run(results, topology, out, paper_out, n_boot, seed, pairs):
    nodes = [n["id"] for n in json.loads(pathlib.Path(topology).read_text())["nodes"]]
    df, dup = load(pathlib.Path(results))
    if not len(df):
        print("No round records found under", results, file=sys.stderr); return 1
    df = df[~df.warmup]
    rl = rep_level(df, nodes)
    summ = summarize(df, rl, n_boot, seed)
    comp = comparisons(df, rl, n_boot, seed)
    hyp = hypotheses(summ, comp)
    svp = sim_vs_prod(comp, summ, pairs)
    tr = trace_summary(pathlib.Path(results))
    out, paper_out = pathlib.Path(out), pathlib.Path(paper_out)
    out.mkdir(parents=True, exist_ok=True)
    flat = lambda d: d.assign(cond=d["cond"].apply(json.dumps)) if "cond" in d else d
    flat(rl).to_csv(out / "rep_level.csv", index=False); flat(summ).to_csv(out / "summary.csv", index=False)
    flat(comp).to_csv(out / "comparisons.csv", index=False); hyp.to_csv(out / "hypotheses.csv", index=False)
    svp.to_csv(out / "sim_vs_prod.csv", index=False)
    md_report(out, summ, comp, hyp, svp, dup)
    write_tex(paper_out, summ, comp, hyp, svp, tr, df)
    figures(paper_out, summ)
    print(f"analysis OK: {len(df)} rounds, {len(summ)} groups, tiers={sorted(summ.tier.unique())}")
    print(hyp.to_string(index=False) if len(hyp) else "")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", default="results"); ap.add_argument("--topology", default="config/topology.json")
    ap.add_argument("--out", default="results/tables"); ap.add_argument("--paper-out", default="paper/generated")
    ap.add_argument("--n-boot", type=int, default=2000); ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--pair", action="append", default=[], help="tierA:condA=tierB:condB (override default same-cond_id pairing)")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    pairs = []
    for p in a.pair:
        l, r = p.split("=")
        pairs.append((*l.split(":", 1), *r.split(":", 1)))
    if a.self_test:
        from synth import write_synthetic
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            write_synthetic(root, "sim", loss_levels=(0, 5), seed=1)
            write_synthetic(root, "prod", loss_levels=(0, 5), seed=2, shift=1.1)
            return run(root, a.topology, root / "tables", root / "tex", 300, a.seed, pairs)
    return run(a.results, a.topology, a.out, a.paper_out, a.n_boot, a.seed, pairs)


if __name__ == "__main__":
    sys.exit(main())
