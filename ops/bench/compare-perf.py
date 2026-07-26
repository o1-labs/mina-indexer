#!/usr/bin/env python3
"""
Phase 3 — performance comparison: mina-indexer vs the archive-node-api.

Both serve the same archive-node-api-compatible GraphQL surface
(`blocks` / `events` / `actions` / `networkState`), so the same query can be
fired at each and the latency / throughput compared apples-to-apples.

For each (endpoint, query) it fires `--requests` requests at `--concurrency`,
records per-request latency, and reports p50 / p95 / p99 / max and throughput.

Stdlib only (urllib + threads) — no install step, runs anywhere Python 3 does.

    ops/bench/compare-perf.py \
        --indexer https://devnet-indexer.gcp.o1test.net/graphql \
        --archive https://devnet-archive-node-api.gcp.o1test.net/ \
        --requests 500 --concurrency 20
"""
import argparse
import json
import re
import statistics
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

# Queries both surfaces answer identically (the indexer is archive-node-api
# compatible). Kept light + read-only; each is a realistic explorer request.
QUERIES = {
    "blocks_recent": "{ blocks(limit: 20, sortBy: BLOCKHEIGHT_DESC) { blockHeight stateHash } }",
    "blocks_page": "{ blocks(limit: 50, sortBy: BLOCKHEIGHT_DESC) { blockHeight stateHash creator } }",
}


def post(url, query, timeout=30):
    """Return (ok, latency_seconds). ok=False on HTTP/GraphQL/transport error."""
    body = json.dumps({"query": query}).encode()
    req = urllib.request.Request(
        url, data=body, headers={"content-type": "application/json"}
    )
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            payload = json.loads(r.read())
        elapsed = time.perf_counter() - start
        ok = r.status == 200 and "errors" not in payload
        return ok, elapsed
    except Exception:
        return False, time.perf_counter() - start


_RSS_RE = re.compile(r"^mina_indexer_process_resident_memory_bytes\s+([0-9.e+]+)", re.M)


def scrape_rss(metrics_url, timeout=10):
    """Resident memory (bytes) from the indexer's /metrics, or None."""
    try:
        with urllib.request.urlopen(metrics_url, timeout=timeout) as r:
            m = _RSS_RE.search(r.read().decode())
        return float(m.group(1)) if m else None
    except Exception:
        return None


class MemSampler(threading.Thread):
    """Polls the indexer's RSS in the background for the whole run, recording the
    peak. The archive-node-api exposes no comparable metric, so memory is
    indexer-only -- what it costs the single process to serve the load."""

    def __init__(self, metrics_url, interval=0.5):
        super().__init__(daemon=True)
        self.metrics_url, self.interval = metrics_url, interval
        self.baseline, self.peak = scrape_rss(metrics_url), 0.0
        self._done = threading.Event()

    def run(self):
        while not self._done.is_set():
            rss = scrape_rss(self.metrics_url)
            if rss is not None:
                self.peak = max(self.peak, rss)
            self._done.wait(self.interval)

    def stop(self):
        self._done.set()


def pct(sorted_vals, p):
    if not sorted_vals:
        return float("nan")
    k = min(len(sorted_vals) - 1, int(round(p / 100.0 * (len(sorted_vals) - 1))))
    return sorted_vals[k]


def run(url, query, n, concurrency):
    # small warmup so cold caches / JIT don't skew the first samples
    for _ in range(min(10, n)):
        post(url, query)

    latencies, errors = [], 0
    wall_start = time.perf_counter()
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        for ok, lat in pool.map(lambda _: post(url, query), range(n)):
            latencies.append(lat)
            errors += 0 if ok else 1
    wall = time.perf_counter() - wall_start

    latencies.sort()
    ms = lambda s: s * 1000.0
    return {
        "n": n,
        "errors": errors,
        "throughput": n / wall if wall > 0 else float("nan"),
        "p50": ms(statistics.median(latencies)),
        "p95": ms(pct(latencies, 95)),
        "p99": ms(pct(latencies, 99)),
        "max": ms(latencies[-1]),
    }


def main():
    ap = argparse.ArgumentParser(description="mina-indexer vs archive-node-api perf")
    ap.add_argument("--indexer", required=True, help="indexer GraphQL URL")
    ap.add_argument("--archive", required=True, help="archive-node-api GraphQL URL")
    ap.add_argument("--requests", type=int, default=500)
    ap.add_argument("--concurrency", type=int, default=20)
    ap.add_argument(
        "--metrics",
        default=None,
        help="indexer /metrics URL for RSS tracking "
        "(default: derived from --indexer by swapping /graphql -> /metrics)",
    )
    args = ap.parse_args()

    metrics_url = args.metrics or args.indexer.replace("/graphql", "/metrics")
    mem = MemSampler(metrics_url)
    if mem.baseline is not None:
        mem.start()

    targets = [("indexer", args.indexer), ("archive", args.archive)]
    print(
        f"# {args.requests} requests @ concurrency {args.concurrency}, "
        f"latency in ms\n"
    )
    header = (
        f"{'query':<18} {'target':<8} {'p50':>8} {'p95':>8} {'p99':>8} "
        f"{'max':>8} {'req/s':>8} {'err':>5}"
    )
    print(header)
    print("-" * len(header))

    summary = {}
    for qname, query in QUERIES.items():
        for tname, url in targets:
            r = run(url, query, args.requests, args.concurrency)
            summary.setdefault(qname, {})[tname] = r
            print(
                f"{qname:<18} {tname:<8} {r['p50']:>8.1f} {r['p95']:>8.1f} "
                f"{r['p99']:>8.1f} {r['max']:>8.1f} {r['throughput']:>8.1f} "
                f"{r['errors']:>5}"
            )
        print()

    # headline: p95 + throughput ratio, indexer vs archive
    print("# indexer vs archive (p95 latency, throughput)")
    for qname, byt in summary.items():
        i, a = byt.get("indexer"), byt.get("archive")
        if not (i and a):
            continue
        lat = a["p95"] / i["p95"] if i["p95"] else float("nan")
        tp = i["throughput"] / a["throughput"] if a["throughput"] else float("nan")
        print(
            f"  {qname:<18} indexer p95 {i['p95']:.0f}ms vs archive {a['p95']:.0f}ms "
            f"({lat:.2f}x faster) | throughput {tp:.2f}x"
        )

    # indexer memory footprint under the load (indexer-only; the archive stack's
    # daemon+archive+PostgreSQL footprint is qualitative -- see README).
    if mem.baseline is not None:
        mem.stop()
        mem.join(timeout=2)
        mib = lambda b: b / (1024 * 1024)
        print(
            f"\n# indexer memory (resident): baseline {mib(mem.baseline):.0f} MiB, "
            f"peak under load {mib(mem.peak):.0f} MiB "
            f"(+{mib(mem.peak - mem.baseline):.0f} MiB)"
        )

    # non-zero exit if either side errored (CI signal)
    total_err = sum(
        r["errors"] for byt in summary.values() for r in byt.values()
    )
    return 1 if total_err else 0


if __name__ == "__main__":
    sys.exit(main())
