# Live demo and viva

## Demo (10-12 min, live system, not slides)
1. `make reproduce-analysis` on a clean clone -> tables/figures regenerate (proves one-command reproduction).
2. Bring up the cluster; show `docker compose ps` / `kubectl -n flreg get pods` and a node's `start` log line (measured `bench_mops`, jurisdiction).
3. Side by side: same federation, `resource_only` vs `policy_resource`, `INELIGIBLE_IDS=n2,n6`. Tail coordinator output / the audit lines: `resource_only` **uses n2/n6** (residency violation); `policy_resource` shows `excluded: jurisdiction_not_permitted`. Show the OPA decision log agreeing (`policy_disagreements = 0`).
4. **Live fault #1 (compliance):** `docker exec participant-n3 touch /tmp/revoked` -> next round n3 is excluded with `license_not_active` (continuous enforcement, Sec. III-E). Restore with `rm`.
5. **Live fault #2 (DDIL):** `scripts/netem.sh apply participant-n3 "delay 300ms loss 10% rate 256kbit"` -> show p99 climbing and the scheduler's RTT-driven re-ranking away from n3.
6. **Live fault #3 (availability trade-off):** stop enough eligible nodes that eligible-up < 3 -> `policy_resource` rounds *fail* (never violate); `resource_only` continues *with* violations. State plainly this is the cost of compliance.
7. Show the **sim-vs-production table** without searching for it.

## Likely viva questions - and the honest answers
* *"Your H1 is trivially true - it's a hard filter."* Yes; it's a verification claim with an independent audit (ground truth + attestation + OPA cross-check). The scientific claims are H2/H2b (cost) and H3, and E3 (trade-off).
* *"Why not turmoil/madsim?"* Deterministic, seeded, shares the real selection code, byte-identical replays (unit-tested); tonic-over-turmoil integration could not be validated in time. Listed as future work. I know what it doesn't model (congestion control, HTTP/2 flow control, scheduler jitter).
* *"Is 30 reps of 20 rounds independent?"* Repetition is the unit; rounds share state, so CIs are cluster bootstrap over repetitions; ratios paired.
* *"Is synthetic data meaningful?"* It provides real compute and real payloads so CPU/network effects are real; it says nothing about financial accuracy or convergence on real data. Paper says so.
* *"Where do sim and prod disagree, and why?"* Read from the table; give the mechanism (jitter, cellular buffering, Pi scheduling); scope the claim accordingly.
* *"Hierarchical/secure aggregation?"* Architecture-level in the paper, **not evaluated**; future work.
* *"Which sovereignty sense?"* Primary legal/regulatory (policy-instrumented), secondary operational, technical out of scope.
* *"Why should I trust the number?"* Pinned image + lockfile, raw data committed, tag = manuscript, `make reproduce-analysis` regenerates every figure.
