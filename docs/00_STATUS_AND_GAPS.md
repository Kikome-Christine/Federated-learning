# Where you stand vs the assessment rubric (and what only YOU can do)

## Rubric mapping

| Criterion (weight) | State now | What closes the gap |
|---|---|---|
| **Experimental design (25 %)** | Was: none. **Now:** pre-registration draft, 30-rep design, paired cluster-bootstrap CIs, effect sizes, mechanical hypothesis verdicts, all tested (`make selftest`, 12 tests). | Read/edit `docs/01_PREREGISTRATION.md`, commit, tag `prereg-v1`, *then* run. |
| **Reproducibility (20 %)** | Was: unbuildable scaffold, no lockfile, unpinned images. **Now:** workspace, `make reproduce*`, Dockerfile with digest pinning, CI. | `make lock`, `make pin`, commit `Cargo.lock` + `docker/digests.env`, push public repo, tag release. **Rust was never compiled in my sandbox** (no toolchain) - see below. |
| **Production validation (20 %)** | **Nothing done; cannot be done by me.** Manifests + procedure ready. | Deploy on Tier B (Pi/k3s) or A; run one trial over a **real** degraded link with `scripts/net_trace.py`. `docs/03_*`. |
| **Sovereignty relevance (15 %)** | Paper never mentions Uganda/DDIL; three senses not distinguished. **Now:** primary/secondary/out-of-scope stated; text drafted in `paper/sections`. | Verify the legal citation you use (DPPA 2019 section). |
| **Communication & defence (20 %)** | Demo + viva prep drafted (`docs/04_*`). | Rehearse it on the real hardware. |

## Things I could not do (be clear with your examiner about these)
1. **Compile or run any Rust.** No toolchain/network in my sandbox. The Python analysis, generators, LaTeX, and dry-run orchestration *are* tested. The Rust is written carefully and reviewed by eye but expect a few compile errors on first `cargo build`. Paste the errors back to me.
2. **Produce results.** There are no numbers in this package. Every table/figure/macro is generated from *your* raw runs. Synthetic fixtures exist only for pipeline tests and are labelled `SYNTHETIC`.
3. Deploy to Tier A/B/C, CRANE Cloud, OpenStack; run the real-network trial; submit the Google Form; give the demo.
4. Verify that every reference in your bibliography exists as cited (several have no author names; see `paper/EDIT_LIST.md`).

## Design decisions you must be able to defend
* Sim tier is a **purpose-built seeded simulator (`flsim`)**, not turmoil/madsim - because I could not compile-test tonic-over-turmoil. It shares the *real selection code*; same seed => byte-identical output (unit-tested). Say so; offer turmoil as future work, or port it if you have time.
* Baselines: exam Table 3.2 lists *architectures*; your paper's contribution is *selection logic*. The harness compares both: strategies (algorithmic arms) and substrates (centralized / compose / k3s / OpenStack / CRANE) via the `substrate` field.
* **Scope cuts to your paper's claims** (do these; see `paper/EDIT_LIST.md`): hierarchical aggregation, secure aggregation, DP, trust scoring, "improved convergence stability / fairness" are not evaluated. Either implement + test them or move them to Architecture/Future Work. The current abstract asserts results that do not exist.

## Priority order (highest grade-value per hour)
1. `cargo build` + fix compile errors -> `cargo test` -> `make smoke`.
2. Edit + commit + tag the pre-registration.
3. `make sim && make analysis` (fast) -> Week-5 presentation material.
4. `make pin lock`, `make emu` (hours) on one fixed host.
5. Tier B/A deployment + **real-network trial** (largest single risk: schedule it early).
6. Paper: cut claims, add sections from `paper/sections`, fix references, fix Fig. 1.
7. Public repo, tagged release, rehearse demo.
