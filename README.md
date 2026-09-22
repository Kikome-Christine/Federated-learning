# fl-reg-orchestration

Regulatory-aware, resource-aware participant orchestration for federated learning
across heterogeneous edge/cloud nodes (Rust). Midterm project for MCS 7101.

> **Status:** harness + analysis + docs complete; **Rust not yet compiled by the author of this
> scaffold - run `cargo build` first.** No experimental results are included until you generate them.
> Read `docs/00_STATUS_AND_GAPS.md` first.

## Layout
```
crates/flproto      protobuf/gRPC definitions (vendored protoc, no system install)
crates/flcommon     PRNG, workload (logistic regression), selection logic  <- shared by real system AND simulator
crates/participant  node: measures its own speed, real local training, attestation
crates/scheduler    per-round probe + policy (native Rust + OPA sidecar, cross-checked) + scoring
crates/coordinator  experiment driver: reps, interleaved strategies, hdrhistogram, JSONL; centralized baseline
crates/flsim        deterministic seeded simulator, same JSONL schema
config/             topology.json (single source of truth), experiments.json (the matrix)
policy/             residency.rego (+ tests for `opa test`)
scripts/            gen_deploy, run_experiments, netem, net_trace, pin_digests
analysis/           stats + analyze.py (tables, figures, verdicts, sim-vs-prod) + tests
deploy/             k3s manifest (generated), VM scripts (OpenStack/bare hosts)
docs/               status, PRE-REGISTRATION, baselines/tiers, production validation, demo/viva
paper/              LaTeX sections + generated tables/figures + EDIT_LIST.md
```

## First hour
```bash
cargo build --release          # fix compile errors (please report them)
cargo test --workspace         # 17 Rust unit tests incl. simulator determinism
make lock                      # creates Cargo.lock  -> COMMIT IT
make selftest                  # analysis pipeline on SYNTHETIC data (not results)
make smoke                     # 3 reps x 5 rounds of E1 on the simulator + analysis
make pin                       # pin base images by digest (needs Docker) -> COMMIT docker/digests.env
```
## Reproduce
| Command | What | Needs |
|---|---|---|
| `make reproduce-analysis` | regenerate every table/figure/macro from committed raw data | Python only |
| `make reproduce-sim` | rebuild, rerun full simulator matrix, analysis | Rust |
| `make reproduce-emu` | Docker + tc/netem tier (hours) | Docker, NET_ADMIN, one fixed host |
| production | `docs/03_PRODUCTION_VALIDATION.md` | Pi/k3s + real link |

**Hardware:** simulator: any laptop (minutes). Emulation: >= 4 cores, 8 GB RAM, Linux with `tc`; expect ~2-4 h for E1+E2+E3 at 30x20 (reduce `rounds` in `config/experiments.json` on weak hardware and disclose). Production: 3-8 Pi 4/5 or Jetson boards.

## Known deviations (disclose in the paper)
`flsim` is a purpose-built simulator (not turmoil/madsim); emulation shapes egress only; hierarchical/secure aggregation not implemented; synthetic data.

## Submission checklist (Exam)
- [ ] pre-registration committed + tag `prereg-v1` BEFORE headline runs  - [ ] `Cargo.lock` + `docker/digests.env` committed
- [ ] raw data in `results/*/raw` committed  - [ ] `make reproduce-analysis` works from a clean clone
- [ ] real-link trial + trace CSV  - [ ] sim-vs-prod table + discrepancy explanation  - [ ] all four baselines run or written justification approved
- [ ] manuscript claims match the tagged release  - [ ] target venue named  - [ ] Google Form submitted  - [ ] demo rehearsed
"# Federated-learning" 
