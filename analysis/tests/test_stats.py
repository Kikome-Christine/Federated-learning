import math, pathlib, sys
import numpy as np
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import stats as S


def test_t_ci_known_values():
    m, lo, hi = S.t_ci([1, 2, 3, 4, 5])
    assert m == 3.0
    assert abs((hi - lo) / 2 - 1.9600 * 0 - 1.963) < 0.01  # t(0.975,4)=2.776 * se(0.7071)=1.963


def test_t_ci_degenerate():
    assert math.isnan(S.t_ci([])[0])
    m, lo, hi = S.t_ci([7.0])
    assert m == 7.0 and math.isnan(lo)


def test_cluster_bootstrap_is_seeded_and_covers_truth():
    rng = np.random.default_rng(0)
    reps = [rng.normal(100, 10, 20) for _ in range(30)]
    a = S.cluster_boot_percentiles(reps, (50, 99), n_boot=400, seed=5)
    b = S.cluster_boot_percentiles(reps, (50, 99), n_boot=400, seed=5)
    assert a == b                                   # determinism
    est, lo, hi = a[50]
    assert lo <= est <= hi and lo < 100 < hi        # true median 100 inside CI


def test_paired_ratio_identical_data_is_one():
    rng = np.random.default_rng(1)
    d = {i: rng.normal(50, 5, 15) for i in range(20)}
    est, lo, hi, n = S.paired_ratio_ci(d, d, 99, n_boot=200, seed=1)
    assert est == 1.0 and lo == 1.0 and hi == 1.0 and n == 20


def test_paired_ratio_detects_slowdown():
    rng = np.random.default_rng(2)
    a = {i: rng.normal(150, 5, 15) for i in range(30)}
    b = {i: rng.normal(100, 5, 15) for i in range(30)}
    est, lo, hi, _ = S.paired_ratio_ci(a, b, 50, n_boot=300, seed=2)
    assert 1.4 < lo < est < hi < 1.6


def test_effect_sizes():
    a, b = [1, 2, 3, 4], [10, 11, 12, 13]
    assert S.cliffs_delta(a, b) == -1.0
    assert S.cliffs_label(-1.0) == "large"
    assert S.cohens_d(a, b) < -5
    assert S.cliffs_delta([1, 2], [1, 2]) == 0.0


def test_jain():
    assert S.jain([5, 5, 5, 5]) == 1.0
    assert abs(S.jain([1, 0, 0, 0]) - 0.25) < 1e-12


def test_decision_rules_three_outcomes():
    assert S.decide_le(1.0, 1.15, 1.2) == "supported"
    assert S.decide_le(1.3, 1.6, 1.2) == "falsified"
    assert S.decide_le(1.1, 1.4, 1.2) == "inconclusive"
    assert S.decide_lt(0.5, 0.9) == "supported"
    assert S.decide_lt(1.1, 1.4) == "falsified"
    assert S.decide_lt(0.8, 1.2) == "inconclusive"
