# Pre-registration (freeze BEFORE the first headline run)

> **Status: DRAFT proposed from your paper.** The hypotheses are *your* intellectual
> commitment, so read, edit, and own them. Then commit this file together with
> `config/experiments.json` and `config/topology.json` and run:
> `git tag prereg-v1 && git push --tags`. Any change after a headline run must be
> logged in section 9 as a deviation - never silently edited. (Exam Sec. 4.1, 4.5.)

## 1. Concept family, sovereignty sense, operating context (Exam Sec. 2)

| Item | Choice | Why |
|---|---|---|
| Concept family | **Hybrid:** *Data localization & geofencing* (policy-aware placement, OPA) + *Lightweight sovereign orchestration* (k3s on Pi-class nodes) | Your Sec. III-D hard eligibility filter is policy-aware placement; your heterogeneous edge nodes are the constrained-orchestration side. A justified hybrid of two is allowed. |
| **Primary** sovereignty sense | **Legal & regulatory** (residency / jurisdictional eligibility), measured with policy-aware instrumentation (OPA decision logs + independent attestation audit) | Your contribution is that regulatory eligibility overrides technical suitability. |
| Secondary | **Operational** (DDIL): behaviour under packet loss, delay, bandwidth caps, node failure | Uganda-class links; tested in E2/E3. |
| Out of scope | **Technical** sovereignty (key custody, provider independence) | State this explicitly in the paper; do not claim it. |
| Operating context | A Uganda-supervised federation of heterogeneous institutions (cloud-class and Pi-class nodes) under data-protection/residency constraints (Uganda DPPA 2019; verify the exact section you cite) | Ties the paper to the assessed Ugandan/constrained context (15 % of grade). |

## 2. Systems compared (all run the same workload)

* **Strategies (algorithmic arms):** `random` (FedAvg-style sampling, policy-blind),
  `resource_only` (same scoring, policy-blind), `policy_resource` (**ours**: hard
  eligibility filter, then the same scoring). Code: `crates/flcommon/src/select.rs`.
* **Mandatory architecture baselines (Exam Table 3.2):** centralized single-node,
  k3s on constrained nodes, OpenStack private cloud, CRANE Cloud. See `docs/02_*`.

## 3. Hypotheses (each falsifiable, each with a mechanical decision rule)

* **H1 - compliance (primary, sovereignty).** `policy_resource` admits **zero**
  ineligible participants in every condition of every tier (ground-truth list
  *and* attestation audit). *Falsified by any single violation.*
  Manipulation check: `random` and `resource_only` **do** violate whenever f > 0
  (if they don't, the design is broken, not the hypothesis proven).
  > Honest note: H1 is nearly true by construction; it is a **verification**
  > claim (the reviewer will say so). Its interesting failure mode is a stale
  > profile / policy race, and the OPA-vs-native disagreement counter.
* **H2 - cost of compliance (headline).** For f in {0.00, 0.25}:
  `p99(policy_resource) / p99(resource_only) <= 1.20` (paired cluster-bootstrap
  95 % CI). *Supported* if CI upper <= 1.20; *falsified* if CI lower > 1.20;
  otherwise *inconclusive*.
* **H2b - predicted break point.** At f = 0.50 the eligible pool equals K = 4 and
  contains one node of every capability class, so we **predict** the ratio
  **exceeds 1.20**. *Supported* if CI lower > 1.20.
* **H3 - operational (secondary).** Under packet loss >= 5 %, resource-aware
  selection beats random: `p99(policy_resource) / p99(random) < 1` (CI upper < 1).
* **Exploratory (not confirmatory, reported regardless):** E3 - with k eligible
  nodes down, `policy_resource` *fails rounds* (never violates) while
  `resource_only`/`random` keep running *with* violations: the
  compliance-vs-availability trade-off. Success rate and violations are reported.

## 4. Variables

| | |
|---|---|
| Independent | selection strategy; ineligible fraction f in {0, .25, .5}; packet loss {0,1,2,5,10} %; added delay {0,100,300} ms; bandwidth cap {none,1000,256} kbit/s (one factor at a time); nodes down {0,2,4} |
| Dependent | round completion latency p50/p95/p99 (ms; hdrhistogram in Rust, re-computed in Python); policy violations; round success rate; bytes/round; participation fairness (Jain); final global loss (emu/prod only) |
| **Held fixed** | N = 8 nodes and their class table; K = 4, min participants 3, deadline 5000 ms; model dim 4096; 1000 samples/node; 3 local epochs; lr 0.5; data seeds; scoring weights (0.35/0.25/0.25/0.15); 20 rounds/rep with 2 warm-up discarded; same container image digest; same host for all emu conditions |
| Randomised | strategy order inside every repetition (seeded); selection/network noise seeds = `base_seed + rep` |

## 5. Sample size and statistics

* **30 independent repetitions per configuration** (unit of independence = repetition;
  rounds inside a repetition are *not* independent).
* Percentiles: pooled over the repetition's post-warm-up rounds; **cluster bootstrap**
  (resample repetitions, 2000 resamples) 95 % CI. Ratios: **paired** cluster bootstrap
  (strategies are interleaved within each repetition with the same seed).
* Means: repetition-level t-interval. Effect sizes: Cohen's d and Cliff's delta.
* Every configuration in `config/experiments.json` is reported, including null/negative.
* No outlier removal. Only declared warm-up rounds are dropped.

## 6. Simulated -> production decision rule (Exam Sec. 5.1)

For the headline quantities (H2 ratio at f = 0.25; H3 ratio at the real-trial
condition) the production result **reproduces** iff (a) same direction relative
to 1.0 **and** (b) relative difference vs the simulated/emulated value <= **20 %**.
If it does not, we scope the claim to *theoretical/simulated*, report the gap and
characterise it. Computed by `analysis/analyze.py` -> `sim_vs_prod.csv`.

## 7. Threats to validity we commit to disclosing

Synthetic data and a linear model (no claim about real financial accuracy);
`flsim` is a purpose-built seeded simulator, not turmoil/madsim; emulation
shapes *egress* only; fixed ineligible pattern in emu/prod (random assignment only
in a sim sensitivity run); the scheduler is a greedy heuristic, not an exact solver
of Eq. (1); hierarchical aggregation, secure aggregation and differential privacy
are **not implemented or evaluated**; real-network conditions cannot be replayed
(trace logged instead); single host for emu (thermal/background noise named).

## 8. Analysis code freeze
`analysis/analyze.py` and `analysis/stats.py` at tag `prereg-v1`.

## 9. Deviations log (append only)
| Date | Change | Reason | Before/after headline run? |
|---|---|---|---|
