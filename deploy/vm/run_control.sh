#!/usr/bin/env bash
# Scheduler (+ OPA) on the control host.
#   ./deploy/vm/run_control.sh "n1=10.0.0.11:50052,n2=10.0.0.12:50052,..."
set -euo pipefail
cd "$(dirname "$0")/../.."
SPEC="${1:?PARTICIPANTS spec id=host:port,...}"; IMAGE="${FLREG_IMAGE:-flreg:dev}"
set -a; source docker/digests.env; set +a
docker rm -f flreg-opa flreg-scheduler >/dev/null 2>&1 || true
docker network create flctl >/dev/null 2>&1 || true
docker run -d --name flreg-opa --network flctl -v "$PWD/policy:/policy:ro" "$OPA_IMAGE" \
  run --server --addr=0.0.0.0:8181 --set=decision_logs.console=true /policy
docker run -d --name flreg-scheduler --network flctl -p 50051:50051 \
  -e PARTICIPANTS="$SPEC" -e POLICY_BACKEND=opa -e OPA_URL=http://flreg-opa:8181 \
  -e ALLOWED_JURISDICTIONS=UG -e ALLOWED_ZONES=ug-central "$IMAGE" scheduler
echo "scheduler on :50051"
