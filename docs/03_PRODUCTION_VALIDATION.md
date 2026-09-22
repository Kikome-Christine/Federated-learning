# Production validation (Exam Sec. 5) - the 20 % you must do on real hardware

Goal: for the headline results, show whether direction and magnitude
**reproduce within +-20 %** on real hardware under a **real** degraded link.

## Step 1 - deploy on the assigned tier
Tier B (3-8 Pi/Jetson) with k3s is the tier the exam says is "most directly relevant". Map the 8 logical nodes onto your boards (2 logical nodes per board is acceptable if you say so). Run **E1** exactly as in `docs/02` with `TIER=prod`, `SUBSTRATE=k3s`. Note board model, RAM, OS, kernel, k3s version, cooling/throttling in `results/prod/env.json` (the runner's `write_env` shows the fields).

## Step 2 - the real degraded-network trial (must NOT be tc/netem)
Pick one that is genuinely available to you (and appropriate to the Ugandan context):
* **Mobile-data uplink (recommended):** put the Pi cluster (or the coordinator+scheduler side) behind a phone hotspot / USB modem on MTN/Airtel mobile data, so control-plane <-> participant traffic crosses a real cellular link. Do the trial at a time/place where the link is genuinely weak (record location/time/signal).
* **Power-constrained node:** run a participant on a battery/power-bank Pi with CPU frequency capped (`cpufreq-set`) - a *deliberate real* limitation, not a synthetic one.
* Whatever you choose, describe it plainly in the paper as uncontrolled and non-replayable.

Run the trace logger on the link **for the entire trial**:
```bash
python3 scripts/net_trace.py --target <far-end-ip> --out results/prod/trace_trial1.csv --duration 3600
# optional throughput samples (iperf3 -s on the far end): --iperf-host <ip> --iperf-every 60
```
Run the coordinator with `COND_ID=E2_real_trial1`, `COND_JSON='{"pattern":"f0.25","real":true}'`, `TIER=prod`. `analyze.py` auto-summarises `trace*.csv` into `tab_trace.tex`.

## Step 3 - make it comparable
Take the trace medians (RTT p50, loss %) and run **sim and emu** with those parameters under a condition you can pair, e.g. median RTT 180 ms, loss 3 %:
```bash
target/release/flsim --label real_match --exp E2 --cond-id E2_matched_to_trial1 --cond-json '{"pattern":"f0.25","loss_pct":3,"delay_ms":90}' \
   --ineligible n2,n6 --loss-pct 3 --delay-ms 90 --out-dir results/sim/raw
python3 analysis/analyze.py --pair "sim:E2_matched_to_trial1=prod:E2_real_trial1"
```
(`delay-ms` is the *added* delay on each shaped egress; RTT ~ class delay + 2 x added.) Document how you derived the parameters **before** looking at the production result.

## Step 4 - apply the decision rule (already coded)
`results/tables/sim_vs_prod.csv` / `tab_sim_vs_prod.tex` lists, for each headline metric: values, relative difference, same-direction, `reproduces` (<=20 %). Then in the paper:
* reproduces -> claim a **practical** contribution and show both numbers side by side;
* does not -> report it, characterise *why* (emulator underestimates jitter? Pi scheduling? cellular buffering?), scope the claim to **simulated** and list it as a limitation. That is publishable (Exam 5.1); hiding it is not.

## Checklist
- [ ] E1 on real hardware, 30 reps  - [ ] one real-link trial with trace CSV  - [ ] sim/emu run matched to trace  - [ ] `env.json` for prod  - [ ] discrepancy table ready for the viva
