#!/usr/bin/env python3
"""
End-to-end ledger fidelity check (issue #19, WS2 — PCB-driven correctness).

Treats the precomputed-block set as the oracle. Every V2 (post-hardfork) block
states the post-block balance of each account it touched in `accounts_accessed`
-- an exact, height-matched ground truth that needs no archive node. This walks
the indexer's own canonical chain, builds that oracle, and asserts the indexer's
`stagedLedgerAccounts` balances agree.

Usage:
    ops/fidelity-check.py --blocks-dir DIR [--gql URL] [--network NET]
                          [--margin N] [--verbose]

Exit status is non-zero if any balance disagrees (CI-friendly).

Notes:
  - Compare below the moving tip (`--margin`, default 40): during an in-progress
    sync the near-tip canonical chain is still settling, which produces spurious
    mismatches on both the indexer and oracle side.
  - V2/hardfork networks only. V1 (pre-hardfork mainnet) blocks carry no
    `accounts_accessed`, so there is no oracle to check against.
"""
import argparse
import glob
import gzip
import json
import os
import sys
import urllib.request

MINA_TOKEN = "wSHV2S4qX9jFsLjQo8r1BsMLH2ZRKsZx6EJd1sbozGPieEC4Jf"


def load_block(path):
    """Load a precomputed block, tolerating gzip and non-utf8 bytes."""
    with open(path, "rb") as fh:
        raw = fh.read()
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    return json.loads(raw.decode("utf-8", errors="replace"))


def gql(url, query):
    req = urllib.request.Request(
        url,
        data=json.dumps({"query": query}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def build_hash_index(blocks_dir, network):
    """hash -> filepath for every `<network>-<height>-<hash>.json` on disk."""
    idx = {}
    for f in glob.glob(os.path.join(blocks_dir, f"{network}-*.json")):
        base = os.path.basename(f)[:-5]  # strip .json
        parts = base.split("-", 2)
        if len(parts) == 3:
            idx[parts[2]] = f
    return idx


def best_tip(url):
    q = (
        "{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true})"
        " { stateHash blockHeight } }"
    )
    b = gql(url, q)["data"]["blocks"][0]
    return b["stateHash"], b["blockHeight"]


def canonical_chain(tip_hash, hash_idx):
    """Ascending list of canonical block files, tip -> genesis via parent links."""
    chain, h = [], tip_hash
    while h in hash_idx:
        d = load_block(hash_idx[h])["data"]
        chain.append(hash_idx[h])
        h = d["protocol_state"]["previous_state_hash"]
    chain.reverse()
    return chain


def build_oracle(files):
    """pk -> last stated MINA balance (nanomina) along the canonical chain."""
    oracle = {}
    for f in files:
        d = load_block(f)["data"]
        for _idx, acct in d.get("accounts_accessed", []):
            if acct.get("token_id") == MINA_TOKEN:
                oracle[acct["public_key"]] = int(acct["balance"])
    return oracle


def indexer_balance(url, pk, height):
    q = (
        '{ stagedLedgerAccounts(query:{publicKey:"%s", blockchain_length:%d})'
        " { balance_nano } }" % (pk, height)
    )
    data = gql(url, q).get("data")
    if not data or not data.get("stagedLedgerAccounts"):
        return None
    return int(data["stagedLedgerAccounts"][0]["balance_nano"])


def file_height(f):
    return int(os.path.basename(f).split("-", 2)[1])


def main():
    ap = argparse.ArgumentParser(description="Indexer ledger fidelity check (#19 WS2)")
    ap.add_argument("--blocks-dir", required=True, help="dir of precomputed block JSON")
    ap.add_argument("--gql", default="http://localhost:8080/graphql")
    ap.add_argument("--network", default="mainnet", help="block filename prefix")
    ap.add_argument("--margin", type=int, default=40, help="compare at tip - margin")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    hash_idx = build_hash_index(args.blocks_dir, args.network)
    if not hash_idx:
        print(f"no {args.network}-*.json blocks in {args.blocks_dir}", file=sys.stderr)
        return 2

    tip_hash, tip_h = best_tip(args.gql)
    target_h = tip_h - args.margin
    files = [f for f in canonical_chain(tip_hash, hash_idx) if file_height(f) <= target_h]
    oracle = build_oracle(files)
    print(
        f"tip {tip_h}; comparing at settled height {target_h} "
        f"({len(files)} canonical blocks, {len(oracle)} MINA accounts)"
    )

    ok = wrong = missing = 0
    mismatches = []
    for pk, want in sorted(oracle.items()):
        got = indexer_balance(args.gql, pk, target_h)
        if got is None:
            missing += 1
            mismatches.append((pk, want, "MISSING"))
        elif got == want:
            ok += 1
        else:
            wrong += 1
            mismatches.append((pk, want, got))

    total = ok + wrong + missing
    print(f"RESULT: {ok}/{total} exact ({wrong} wrong, {missing} missing)")
    if args.verbose or mismatches:
        for pk, want, got in mismatches[:50]:
            g = got if isinstance(got, str) else f"{got / 1e9:.9f}"
            print(f"  {pk} want={want / 1e9:.9f} got={g}")

    return 0 if (wrong == 0 and missing == 0) else 1


if __name__ == "__main__":
    sys.exit(main())
