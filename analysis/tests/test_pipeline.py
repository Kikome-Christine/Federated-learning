import json, pathlib, sys, tempfile
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import analyze as A
from synth import write_synthetic

TOPO = str(pathlib.Path(__file__).resolve().parents[2] / "config/topology.json")


def _run(root, **kw):
    return A.run(root, TOPO, root / "tables", root / "tex", 150, 1, kw.get("pairs", []))


def test_end_to_end_outputs_and_h1(tmp_path):
    write_synthetic(tmp_path, "sim", seed=1)
    write_synthetic(tmp_path, "prod", seed=2, shift=1.1)
    assert _run(tmp_path) == 0
    for f in ("summary.csv", "comparisons.csv", "hypotheses.csv", "sim_vs_prod.csv", "REPORT.md"):
        assert (tmp_path / "tables" / f).exists(), f
    for f in ("macros.tex", "tab_e1_sim.tex", "tab_ratios_prod.tex", "tab_sim_vs_prod.tex", "tab_hypotheses_sim.tex"):
        assert (tmp_path / "tex" / f).exists(), f
    assert list((tmp_path / "tex").glob("fig_e2_p99_sim.pdf"))
    import pandas as pd
    h = pd.read_csv(tmp_path / "tables/hypotheses.csv")
    assert (h[h.hyp == "H1"].verdict == "supported").all()


def test_violation_is_detected_when_policy_strategy_leaks(tmp_path):
    raw = write_synthetic(tmp_path, "sim", seed=3, reps=5)
    f = next(raw.glob("E1_*.jsonl"))
    lines = [json.loads(l) for l in f.read_text().splitlines()]
    for r in lines:                          # inject a leak: policy_resource used ineligible n2
        if r["strategy"] == "policy_resource" and r["rep"] == 0 and r["round"] == 5:
            r["responded"].append({"id": "n2", "jurisdiction": "XX", "license": "active"})
    f.write_text("\n".join(json.dumps(r) for r in lines) + "\n")
    _run(tmp_path)
    import pandas as pd
    h = pd.read_csv(tmp_path / "tables/hypotheses.csv")
    assert (h[h.hyp == "H1"].verdict == "FALSIFIED").all()


def test_warmup_rounds_are_excluded(tmp_path):
    raw = write_synthetic(tmp_path, "sim", seed=4, reps=3, rounds=6)
    df, _ = A.load(tmp_path)
    assert df.warmup.sum() > 0
    df = df[~df.warmup]
    assert df["round"].min() == 2


def test_duplicate_rows_are_reported(tmp_path, capsys):
    raw = write_synthetic(tmp_path, "sim", seed=5, reps=2, rounds=3)
    f = next(raw.glob("E1_*.jsonl"))
    f.write_text(f.read_text() * 2)
    _, dup = A.load(tmp_path)
    assert dup > 0
    assert "duplicate" in capsys.readouterr().err
