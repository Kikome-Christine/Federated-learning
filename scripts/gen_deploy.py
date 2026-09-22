#!/usr/bin/env python3
"""Generate docker-compose.yml and deploy/k3s/flreg.yaml from config/topology.json.

Single source of truth => the compose testbed, the k3s edge cluster and the
simulator can never silently drift apart.
"""
import copy, json, pathlib, sys
import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent
topo = json.loads((ROOT / "config/topology.json").read_text())
model, classes, nodes = topo["model"], topo["classes"], topo["nodes"]
pol = topo["policy"]
sched_cfg = topo["scheduler"]


def participants_spec(host_fmt):
    return ",".join(f"{n['id']}={host_fmt.format(id=n['id'])}:50052" for n in nodes)


def model_env():
    return {
        "MODEL_DIM": str(model["dim"]),
        "N_SAMPLES": str(model["n_samples"]),
        "DATA_SEED": str(model["data_seed"]),
    }


# ------------------------------------------------------------------ compose
def gen_compose():
    build = {
        "context": ".",
        "dockerfile": "docker/Dockerfile",
        "args": {
            "RUST_IMAGE": "${RUST_IMAGE:-rust:1.90-bookworm}",
            "RUNTIME_IMAGE": "${RUNTIME_IMAGE:-debian:bookworm-slim}",
        },
    }
    svcs = {}
    svcs["opa"] = {
        "image": "${OPA_IMAGE:-openpolicyagent/opa:1.4.2}",
        "command": ["run", "--server", "--addr=0.0.0.0:8181",
                    "--set=decision_logs.console=true", "/policy"],
        "volumes": ["./policy:/policy:ro"],
        "networks": ["flnet"],
    }
    for n in nodes:
        c = classes[n["class"]]
        env = {
            "PARTICIPANT_ID": n["id"],
            "TIER": c["tier"],
            "CPU_CORES": str(max(1, int(round(c["cpus"])))),
            "MEMORY_MB": str(c["mem_mb"]),
            "BANDWIDTH_MBPS": str(c["link_rate_mbit"]),
            "INELIGIBLE_IDS": "${INELIGIBLE_IDS:-}",
            "JURISDICTION": pol["allowed_jurisdictions"][0],
            "RESIDENCY_ZONE": pol["allowed_zones"][0],
            **model_env(),
        }
        svcs[f"participant-{n['id']}"] = {
            "image": "flreg:dev",
            "command": ["participant"],
            "hostname": f"participant-{n['id']}",
            "cpus": c["cpus"],
            "mem_limit": f"{c['mem_mb']}m",
            "cap_add": ["NET_ADMIN"],
            "labels": {"flreg.class": n["class"], "flreg.node": n["id"]},
            "environment": env,
            "networks": ["flnet"],
        }
    svcs["scheduler"] = {
        "image": "flreg:dev",
        "build": build,
        "command": ["scheduler"],
        "hostname": "scheduler",
        "cap_add": ["NET_ADMIN"],
        "environment": {
            "PARTICIPANTS": participants_spec("participant-{id}"),
            "POLICY_BACKEND": "${POLICY_BACKEND:-opa}",
            "OPA_URL": "http://opa:8181",
            "ALLOWED_JURISDICTIONS": ",".join(pol["allowed_jurisdictions"]),
            "ALLOWED_ZONES": ",".join(pol["allowed_zones"]),
            "PROBE_TIMEOUT_MS": str(sched_cfg["probe_timeout_ms"]),
            "W_COMPUTE": str(sched_cfg["weights"]["compute"]),
            "W_BANDWIDTH": str(sched_cfg["weights"]["bandwidth"]),
            "W_LATENCY": str(sched_cfg["weights"]["latency"]),
            "W_AVAILABILITY": str(sched_cfg["weights"]["availability"]),
        },
        "depends_on": ["opa"],
        "networks": ["flnet"],
    }
    svcs["coordinator"] = {
        "image": "flreg:dev",
        "command": ["coordinator"],
        "hostname": "coordinator",
        "profiles": ["run"],
        "cap_add": ["NET_ADMIN"],
        "environment": {
            "SCHEDULER_URL": "http://scheduler:50051",
            "NODE_IDS": ",".join(n["id"] for n in nodes),
            "OUT_DIR": "/results/raw",
            "LOCAL_EPOCHS": str(model["local_epochs"]),
            "LEARNING_RATE": str(model["lr"]),
            "EVAL_SEED": str(model["eval_seed"]),
            "N_EVAL": str(model["n_eval"]),
            "GIT_SHA": "${GIT_SHA:-unknown}",
            **model_env(),
        },
        "volumes": ["./results:/results"],
        "networks": ["flnet"],
    }
    doc = {"name": "flreg", "services": copy.deepcopy(svcs),
           "networks": {"flnet": {"driver": "bridge"}}}
    return doc


# --------------------------------------------------------------------- k3s
def gen_k3s():
    docs = [{"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "flreg"}}]
    labels = lambda app: {"app": app, "app.kubernetes.io/part-of": "flreg"}

    def deployment(name, cmd, env, port, resources=None, node_selector=None, extra_pod=None):
        pod = {
            "containers": [{
                "name": name, "image": "${FLREG_IMAGE}", "imagePullPolicy": "IfNotPresent",
                "command": [cmd],
                "env": [{"name": k, "value": str(v)} for k, v in env.items()],
                "ports": [{"containerPort": port}],
                **({"resources": resources} if resources else {}),
            }],
        }
        if node_selector:
            pod["nodeSelector"] = node_selector
        if extra_pod:
            pod.update(extra_pod)
        return {
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": name, "namespace": "flreg", "labels": labels(name)},
            "spec": {"replicas": 1, "selector": {"matchLabels": {"app": name}},
                     "template": {"metadata": {"labels": labels(name)}, "spec": pod}},
        }

    def service(name, port):
        return {"apiVersion": "v1", "kind": "Service",
                "metadata": {"name": name, "namespace": "flreg"},
                "spec": {"selector": {"app": name}, "ports": [{"port": port, "targetPort": port}]}}

    for n in nodes:
        c = classes[n["class"]]
        name = f"participant-{n['id']}"
        env = {
            "PARTICIPANT_ID": n["id"], "TIER": c["tier"],
            "CPU_CORES": max(1, int(round(c["cpus"]))), "MEMORY_MB": c["mem_mb"],
            "BANDWIDTH_MBPS": c["link_rate_mbit"], "INELIGIBLE_IDS": "${INELIGIBLE_IDS}",
            "JURISDICTION": pol["allowed_jurisdictions"][0], "RESIDENCY_ZONE": pol["allowed_zones"][0],
            **model_env(),
        }
        res = {"limits": {"cpu": f"{int(c['cpus'] * 1000)}m", "memory": f"{c['mem_mb']}Mi"},
               "requests": {"cpu": f"{int(c['cpus'] * 500)}m", "memory": f"{c['mem_mb'] // 2}Mi"}}
        # Optionally pin classes to labelled Pis:  kubectl label node <pi> flreg/class=edge_small
        docs += [deployment(name, "participant", env, 50052, res), service(name, 50052)]

    docs.append({"apiVersion": "v1", "kind": "ConfigMap",
                 "metadata": {"name": "opa-policy", "namespace": "flreg"},
                 "data": {"residency.rego": (ROOT / "policy/residency.rego").read_text()}})
    opa = deployment("opa", "opa", {}, 8181)
    opa["spec"]["template"]["spec"]["containers"][0].update({
        "image": "${OPA_IMAGE}", "command": None,
        "args": ["run", "--server", "--addr=0.0.0.0:8181", "--set=decision_logs.console=true", "/policy"],
        "volumeMounts": [{"name": "policy", "mountPath": "/policy"}]})
    del opa["spec"]["template"]["spec"]["containers"][0]["command"]
    opa["spec"]["template"]["spec"]["volumes"] = [{"name": "policy", "configMap": {"name": "opa-policy"}}]
    docs += [opa, service("opa", 8181)]

    docs += [deployment("scheduler", "scheduler", {
        "PARTICIPANTS": participants_spec("participant-{id}.flreg.svc.cluster.local"),
        "POLICY_BACKEND": "opa", "OPA_URL": "http://opa.flreg.svc.cluster.local:8181",
        "ALLOWED_JURISDICTIONS": ",".join(pol["allowed_jurisdictions"]),
        "ALLOWED_ZONES": ",".join(pol["allowed_zones"]),
        "PROBE_TIMEOUT_MS": sched_cfg["probe_timeout_ms"]}, 50051), service("scheduler", 50051)]
    # Coordinator job template: copy, set COND_ID/COND_JSON/STRATEGIES, `kubectl apply`.
    docs.append({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "coordinator-${RUN_LABEL}", "namespace": "flreg"},
        "spec": {"backoffLimit": 0, "template": {"spec": {"restartPolicy": "Never",
            "containers": [{"name": "coordinator", "image": "${FLREG_IMAGE}", "command": ["coordinator"],
                "env": [{"name": k, "value": str(v)} for k, v in {
                    "SCHEDULER_URL": "http://scheduler.flreg.svc.cluster.local:50051",
                    "NODE_IDS": ",".join(n["id"] for n in nodes),
                    "OUT_DIR": "/results/raw", "TIER": "prod", "SUBSTRATE": "k3s",
                    "RUN_LABEL": "${RUN_LABEL}", "EXP": "${EXP}", "COND_ID": "${COND_ID}",
                    "COND_JSON": "${COND_JSON}", "INELIGIBLE_IDS": "${INELIGIBLE_IDS}",
                    "LOCAL_EPOCHS": model["local_epochs"], "LEARNING_RATE": model["lr"],
                    "EVAL_SEED": model["eval_seed"], "N_EVAL": model["n_eval"], **model_env()}.items()],
                "volumeMounts": [{"name": "results", "mountPath": "/results"}]}],
            # hostPath keeps raw data on the node; `kubectl cp` it out afterwards.
            "volumes": [{"name": "results", "hostPath": {"path": "/var/flreg-results", "type": "DirectoryOrCreate"}}]}}},
    })
    return docs


def main():
    compose = gen_compose()
    (ROOT / "docker-compose.yml").write_text(
        "# GENERATED by scripts/gen_deploy.py from config/topology.json -- do not edit.\n"
        + yaml.safe_dump(compose, sort_keys=False, width=100))
    k = gen_k3s()
    (ROOT / "deploy/k3s/flreg.yaml").write_text(
        "# GENERATED by scripts/gen_deploy.py -- apply with: envsubst < deploy/k3s/flreg.yaml | kubectl apply -f -\n"
        + yaml.safe_dump_all(k, sort_keys=False, width=100))
    print(f"wrote docker-compose.yml ({len(compose['services'])} services) and deploy/k3s/flreg.yaml ({len(k)} objects)")


if __name__ == "__main__":
    sys.exit(main())
