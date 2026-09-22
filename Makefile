# One entry point per claim in the paper.
#   make reproduce-analysis  -> regenerate EVERY table, figure and macro in the paper
#                               from the committed raw data (minutes, no Docker/Rust needed)
#   make reproduce-sim       -> rebuild flsim, re-run the whole simulator matrix, then analysis
#   make reproduce-emu       -> Docker + tc/netem tier (hours; needs Docker, NET_ADMIN)
#   make reproduce           -> reproduce-sim (the fully automatic path) + analysis
PY ?= python3

.PHONY: help gen lock pin build test sim emu analysis reproduce reproduce-analysis reproduce-sim reproduce-emu smoke selftest clean-results

help:
	@grep -E '^#' Makefile | head -8

gen:                       ## regenerate docker-compose.yml + k3s manifest from config/topology.json
	$(PY) scripts/gen_deploy.py

lock:                      ## create Cargo.lock (COMMIT IT: pinned dependency versions)
	cargo generate-lockfile

pin:                       ## pin base images by digest (COMMIT docker/digests.env)
	./scripts/pin_digests.sh

build:
	cargo build --release --locked --workspace

test:                      ## Rust unit tests + Python tests
	cargo test --workspace
	$(PY) -m pytest -q analysis/tests

sim: build
	$(PY) scripts/run_experiments.py --tier sim

emu:
	$(PY) scripts/run_experiments.py --tier emu

analysis:
	$(PY) analysis/analyze.py --results results --topology config/topology.json --out results/tables --paper-out paper/generated

selftest:                  ## pipeline check on SYNTHETIC data (never used for results)
	$(PY) analysis/analyze.py --self-test

smoke: build               ## 3 reps x 5 rounds of E1 on the simulator, entirely in /tmp (never touches ./results)
	rm -rf /tmp/flreg_smoke_results
	$(PY) scripts/run_experiments.py --tier sim --quick --exp E1 --results-root /tmp/flreg_smoke_results
	$(PY) analysis/analyze.py --results /tmp/flreg_smoke_results --topology config/topology.json --out /tmp/flreg_smoke --paper-out /tmp/flreg_smoke_tex

reproduce-analysis: analysis
reproduce-sim: sim analysis
reproduce-emu: emu analysis
reproduce: reproduce-sim

clean-results:             ## careful: deletes raw data
	rm -rf results/sim/raw results/emu/raw results/tables/* paper/generated/*
