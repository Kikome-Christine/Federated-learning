# The four mandatory baselines and the three infrastructure tiers

Exam Table 3.2 lists *architectures*. The same workload (same binaries, same
config, same metrics, same JSONL schema) is deployed on each; the `substrate`
field in every record says where it ran. Compare **only** identical conditions
(use E1, pattern f0.25, no impairment, strategies `policy_resource` and `resource_only`).

| Baseline (Exam) | `substrate` | Tier | How |
|---|---|---|---|
| Centralized single node | `centralized` | any | `MODE=centralized` (automatic in `make emu`, label `E0_centralized_f0.25`). All institutions' raw data is pooled in one process -> by construction it "violates" residency for every ineligible node. It is the *floor*. |
| Lightweight K8s (k3s) | `k3s` | **B** (Pi/Jetson) or A | `deploy/k3s/flreg.yaml` (below) |
| OpenStack private cloud | `openstack` | **A** | one VM per node via `deploy/vm/*.sh` (below) |
| CRANE Cloud (managed PaaS) | `crane` | **C** | deploy images as CRANE apps (below) |
| *Ours on a single host* | `compose` | laptop/server | `make emu` (netem sweeps live here) |

## k3s (Tier B or A)
```bash
# build + push the image to a registry the cluster can pull from (arm64 for Pis):
docker buildx build --platform linux/arm64,linux/amd64 -f docker/Dockerfile -t <registry>/flreg:<tag> --push .
export FLREG_IMAGE=<registry>/flreg:<tag> OPA_IMAGE=$(grep OPA_IMAGE docker/digests.env | cut -d= -f2) INELIGIBLE_IDS="n2,n6"
envsubst < deploy/k3s/flreg.yaml | kubectl apply -f -
# one coordinator run == one condition (edit COND_ID / STRATEGIES etc. as env for the Job):
export RUN_LABEL=k3s_E1_f0.25 EXP=E1 COND_ID=E1_f0.25_loss0_delay0_ratenone COND_JSON='{"pattern":"f0.25"}'
# (add STRATEGIES / REPS / ROUNDS / BASE_SEED env to the Job spec if you change defaults)
envsubst < deploy/k3s/flreg.yaml | kubectl apply -f - ; kubectl -n flreg logs -f job/coordinator-$RUN_LABEL
# raw data is on the node at /var/flreg-results/raw/*.jsonl -> copy to results/prod/raw/
```
Label Pis by class if you want stable placement: `kubectl label node <pi> flreg/class=edge_small` and add a `nodeSelector` to the matching Deployment. **Use the same condition IDs as sim/emu** so `sim_vs_prod` can join them. Chaos Mesh (`NetworkChaos`) gives you netem-equivalent shaping inside k3s (this is *emulation*, not the real-link trial).

## OpenStack (Tier A - ask CoCIS for the project/quota; Kolla-Ansible is the operator's side, not yours)
```bash
# on each of 8 VMs (docker installed, image pulled):  ./deploy/vm/run_node.sh n3 "n2,n6"
# on a control VM:                                    ./deploy/vm/run_control.sh "n1=10.0.0.11:50052,..."
# coordinator from the control VM, TIER=prod SUBSTRATE=openstack, same env as compose runs.
```

## CRANE Cloud (Tier C) - **check feasibility on day 1**
Deploy the same image as apps from the console/CLI (docs.cranecloud.io; per its docs apps are container images with env vars set in the console - verify current UI/CLI syntax). Deploy 8 participant apps (`participant` command, port 50052, env from `topology.json`) and 1 scheduler app (`PARTICIPANTS=` with the apps' internal/external URLs). Run the coordinator from your machine/Tier A with `SCHEDULER_URL=<scheduler URL>`, `SUBSTRATE=crane`.
**Known risks to test immediately:**
1. **gRPC needs HTTP/2 end-to-end.** If CRANE's ingress terminates only HTTP/1.1, tonic will fail. Test with `grpcurl` first. Fallback: state it as a *finding* (managed PaaS ingress cannot carry gRPC) and run only what works - do **not** silently drop the baseline (needs written approval from the program lead).
2. No `NET_ADMIN`/netem on a PaaS: E1 only, no sweeps; latency then includes the public Internet path - report that.
3. Apps scale-to-zero/restart differently; keep replicas = 1 and note it.
Cite Bainomugisha & Mwotil (2021) in related work as a *sovereignty-oriented architecture*, not only as a deployment convenience (Exam Sec. 3.4).

## Fixed comparison protocol across substrates
Same image digest, same `topology.json`, E1 f0.25 no impairment, 30 reps x 20 rounds, strategies interleaved, coordinator on a **separate** host from participants (state where). Report `p50/p99` with CI + violations per substrate: `paper/generated/tab_substrates.tex`.
