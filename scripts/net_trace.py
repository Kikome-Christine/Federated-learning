#!/usr/bin/env python3
"""Log a REAL network trace (RTT + loss over time) during a production trial.



    python3 scripts/net_trace.py --target 8.8.8.8 --out results/prod/trace_trial1.csv
    # optional bandwidth samples every 60 s (needs `iperf3 -s` on --iperf-host):
    python3 scripts/net_trace.py --target <coordinator-ip> --iperf-host <coordinator-ip> --out ...

Output CSV: t_unix_ms, kind, rtt_ms, lost, mbit_s
"""
import argparse, csv, re, subprocess, sys, time


def ping_once(target, timeout_s):
    try:
        p = subprocess.run(["ping", "-n", "-c", "1", "-W", str(timeout_s), target],
                           capture_output=True, text=True, timeout=timeout_s + 2)
    except subprocess.TimeoutExpired:
        return None
    m = re.search(r"time[=<]([\d.]+)\s*ms", p.stdout)
    return float(m.group(1)) if m else None


def iperf_once(host, secs=3):
    try:
        p = subprocess.run(["iperf3", "-c", host, "-t", str(secs), "-J"], capture_output=True, text=True, timeout=secs + 15)
        import json
        return json.loads(p.stdout)["end"]["sum_received"]["bits_per_second"] / 1e6
    except Exception:
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--interval", type=float, default=1.0)
    ap.add_argument("--duration", type=float, default=0, help="seconds; 0 = until Ctrl-C")
    ap.add_argument("--iperf-host")
    ap.add_argument("--iperf-every", type=float, default=60.0)
    a = ap.parse_args()
    t_end = time.time() + a.duration if a.duration else float("inf")
    last_iperf = 0.0
    with open(a.out, "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["t_unix_ms", "kind", "rtt_ms", "lost", "mbit_s"])
        try:
            while time.time() < t_end:
                t0 = time.time()
                rtt = ping_once(a.target, 2)
                w.writerow([int(t0 * 1000), "ping", "" if rtt is None else rtt, int(rtt is None), ""])
                if a.iperf_host and t0 - last_iperf >= a.iperf_every:
                    mb = iperf_once(a.iperf_host)
                    w.writerow([int(time.time() * 1000), "iperf", "", "", "" if mb is None else round(mb, 3)])
                    last_iperf = t0
                fh.flush()
                time.sleep(max(0.0, a.interval - (time.time() - t0)))
        except KeyboardInterrupt:
            pass
    print("trace written to", a.out, file=sys.stderr)


if __name__ == "__main__":
    main()
