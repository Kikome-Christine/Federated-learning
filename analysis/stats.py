"""Statistics used by the analysis. Small, dependency-light, unit-tested.

Unit of independence = the REPETITION (a full fresh run of R rounds), not the
round: rounds inside a repetition share state (availability EWMA, model), so
they are not independent. All bootstrap CIs therefore resample repetitions
(cluster bootstrap) and pool the rounds inside them.
"""
from __future__ import annotations

import math
from typing import Sequence

import numpy as np
from scipy import stats as sps

NAN = float("nan")


def t_ci(x: Sequence[float], conf: float = 0.95):
    """(mean, lo, hi) using the t-distribution over repetition-level values."""
    x = np.asarray([v for v in x if not (v is None or math.isnan(v))], dtype=float)
    n = len(x)
    if n == 0:
        return NAN, NAN, NAN
    m = float(x.mean())
    if n < 2:
        return m, NAN, NAN
    se = float(x.std(ddof=1)) / math.sqrt(n)
    h = float(sps.t.ppf(0.5 + conf / 2, n - 1)) * se
    return m, m - h, m + h


def _pool(reps: list, idx) -> np.ndarray:
    return np.concatenate([reps[i] for i in idx])


def pooled_percentiles(reps: list, qs=(50, 95, 99)):
    allv = np.concatenate(reps)
    return [float(np.percentile(allv, q)) for q in qs]


def cluster_boot_percentiles(reps: list, qs=(50, 95, 99), n_boot=2000, seed=0, conf=0.95):
    """Point estimate + percentile-bootstrap CI for pooled percentiles,
    resampling whole repetitions with replacement.
    Returns {q: (est, lo, hi)}."""
    est = pooled_percentiles(reps, qs)
    n = len(reps)
    if n < 2:
        return {q: (e, NAN, NAN) for q, e in zip(qs, est)}
    rng = np.random.default_rng(seed)
    boots = np.empty((n_boot, len(qs)))
    for b in range(n_boot):
        idx = rng.integers(0, n, size=n)
        boots[b] = np.percentile(_pool(reps, idx), qs)
    a = (1 - conf) / 2 * 100
    lo, hi = np.percentile(boots, [a, 100 - a], axis=0)
    return {q: (est[i], float(lo[i]), float(hi[i])) for i, q in enumerate(qs)}


def paired_ratio_ci(reps_a: dict, reps_b: dict, q=99, n_boot=2000, seed=0, conf=0.95):
    """Ratio of pooled q-th percentiles A/B with a PAIRED cluster bootstrap.
    reps_* : {rep_index: np.ndarray}. Pairing is by rep index (strategies are
    interleaved inside each repetition with the same seed). Returns (est, lo, hi, n_paired)."""
    keys = sorted(set(reps_a) & set(reps_b))
    n = len(keys)
    if n == 0:
        return NAN, NAN, NAN, 0
    A = [reps_a[k] for k in keys]
    B = [reps_b[k] for k in keys]
    est = float(np.percentile(np.concatenate(A), q) / np.percentile(np.concatenate(B), q))
    if n < 2:
        return est, NAN, NAN, n
    rng = np.random.default_rng(seed)
    r = np.empty(n_boot)
    for b in range(n_boot):
        idx = rng.integers(0, n, size=n)
        r[b] = np.percentile(_pool(A, idx), q) / np.percentile(_pool(B, idx), q)
    a = (1 - conf) / 2 * 100
    lo, hi = np.percentile(r, [a, 100 - a])
    return est, float(lo), float(hi), n


def cohens_d(a: Sequence[float], b: Sequence[float]) -> float:
    a, b = np.asarray(a, float), np.asarray(b, float)
    if len(a) < 2 or len(b) < 2:
        return NAN
    sp = math.sqrt(((len(a) - 1) * a.var(ddof=1) + (len(b) - 1) * b.var(ddof=1)) / (len(a) + len(b) - 2))
    return float((a.mean() - b.mean()) / sp) if sp > 0 else NAN


def cliffs_delta(a: Sequence[float], b: Sequence[float]) -> float:
    """P(a>b) - P(a<b); in [-1, 1]. |d|<0.147 negligible, <0.33 small, <0.474 medium, else large."""
    a, b = np.asarray(a, float), np.asarray(b, float)
    if len(a) == 0 or len(b) == 0:
        return NAN
    gt = (a[:, None] > b[None, :]).sum()
    lt = (a[:, None] < b[None, :]).sum()
    return float((gt - lt) / (len(a) * len(b)))


def cliffs_label(d: float) -> str:
    if math.isnan(d):
        return "n/a"
    d = abs(d)
    return "negligible" if d < 0.147 else "small" if d < 0.33 else "medium" if d < 0.474 else "large"


def jain(counts: Sequence[float]) -> float:
    c = np.asarray(counts, float)
    s2 = float((c ** 2).sum())
    return float(c.sum() ** 2 / (len(c) * s2)) if s2 > 0 and len(c) > 0 else NAN


def decide_le(lo: float, hi: float, thr: float) -> str:
    """H: ratio <= thr. supported if CI upper <= thr; falsified if CI lower > thr; else inconclusive."""
    if math.isnan(hi) or math.isnan(lo):
        return "inconclusive"
    return "supported" if hi <= thr else "falsified" if lo > thr else "inconclusive"


def decide_lt(lo: float, hi: float, thr: float = 1.0) -> str:
    """H: ratio < thr. supported if CI upper < thr; falsified if CI lower >= thr."""
    if math.isnan(hi) or math.isnan(lo):
        return "inconclusive"
    return "supported" if hi < thr else "falsified" if lo >= thr else "inconclusive"
