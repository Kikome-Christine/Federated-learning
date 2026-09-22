#!/usr/bin/env bash
# Manual helper for tc/netem on a running compose container.
#   scripts/netem.sh apply participant-n3 "delay 100ms loss 5% rate 1000kbit"
#   scripts/netem.sh clear participant-n3
#   scripts/netem.sh show  participant-n3
set -euo pipefail
cd "$(dirname "$0")/.."
cmd="${1:?apply|clear|show}"; svc="${2:?service name}"
DC="docker compose --env-file docker/digests.env"
case "$cmd" in
  apply) $DC exec -T "$svc" tc qdisc replace dev eth0 root netem ${3:?netem args} ;;
  clear) $DC exec -T "$svc" tc qdisc del dev eth0 root 2>/dev/null || true ;;
  show)  $DC exec -T "$svc" tc qdisc show dev eth0 ;;
  *) echo "unknown command $cmd"; exit 2 ;;
esac
