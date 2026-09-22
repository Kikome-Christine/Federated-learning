#!/usr/bin/env bash
# Start ONE participant on a VM / bare host (OpenStack VM, Pi, CRANE-adjacent host).
#   ./deploy/vm/run_node.sh n3 "n2,n6"      # node id, comma list of INELIGIBLE ids (pattern)
# Resource class and limits are read from config/topology.json (single source of truth).
set -euo pipefail
cd "$(dirname "$0")/../.."
ID="${1:?node id, e.g. n3}"; INEL="${2:-}"; IMAGE="${FLREG_IMAGE:-flreg:dev}"
read -r CPUS MEM RATE TIER < <(python3 - "$ID" <<'PY'
import json,sys
t=json.load(open("config/topology.json")); n=[x for x in t["nodes"] if x["id"]==sys.argv[1]][0]; c=t["classes"][n["class"]]
print(c["cpus"], c["mem_mb"], c["link_rate_mbit"], c["tier"])
PY
)
MODEL_DIM=$(python3 -c "import json;print(json.load(open('config/topology.json'))['model']['dim'])")
docker rm -f "participant-$ID" >/dev/null 2>&1 || true
docker run -d --name "participant-$ID" --cpus "$CPUS" --memory "${MEM}m" --cap-add NET_ADMIN -p 50052:50052 \
  -e PARTICIPANT_ID="$ID" -e TIER="$TIER" -e BANDWIDTH_MBPS="$RATE" -e INELIGIBLE_IDS="$INEL" -e MODEL_DIM="$MODEL_DIM" \
  "$IMAGE" participant
echo "participant-$ID up on :50052 (cpus=$CPUS mem=${MEM}m)"
