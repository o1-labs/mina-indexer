#!/usr/bin/env python3
"""
Devnet ledger fidelity check for issue #86.

Oracle: each canonical block states the post-block balance of every account it
touched in `accounts_accessed` (nanomina string). Walk the canonical chain
(parents from the indexer's best tip, via block files) ascending and record each
MINA-token account's last stated balance. That is a height-matched, indexer-
independent oracle.

Compare against the indexer's stagedLedgerAccounts(publicKey, blockchain_length)
balance_nano at the tip height.
"""
import json, glob, os, sys, gzip, urllib.request


def load_block(f):
    """Load a block file, tolerating gzip and non-utf8 bytes."""
    with open(f, "rb") as fh:
        raw = fh.read()
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    return json.loads(raw.decode("utf-8", errors="replace"))

BLOCKS_DIR = "data/mainnet-blocks"
GQL = "http://localhost:8080/graphql"
MINA_TOKEN = "wSHV2S4qX9jFsLjQo8r1BsMLH2ZRKsZx6EJd1sbozGPieEC4Jf"
HOLDOUT = "n/a"


def gql(query):
    req = urllib.request.Request(
        GQL, data=json.dumps({"query": query}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def build_hash_index():
    """hash -> filepath for every block on disk."""
    idx = {}
    for f in glob.glob(os.path.join(BLOCKS_DIR, "mainnet-*.json")):
        # mainnet-<height>-<hash>.json
        base = os.path.basename(f)[:-5]
        h = base.split("-", 2)[2]
        idx[h] = f
    return idx


def best_tip():
    """Canonical tip (state_hash, height) from the indexer."""
    q = ('{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true})'
         ' { stateHash blockHeight } }')
    d = gql(q)
    b = d["data"]["blocks"][0]
    return b["stateHash"], b["blockHeight"]


def canonical_chain(tip_hash, hash_idx):
    """Walk parents from tip via block files. Returns ascending list of files."""
    chain = []
    h = tip_hash
    while h in hash_idx:
        f = hash_idx[h]
        d = load_block(f)["data"]
        chain.append((d["protocol_state"], f))
        h = d["protocol_state"]["previous_state_hash"]
    chain.reverse()
    return [f for _, f in chain]


def build_oracle(files):
    """pk -> last stated MINA balance (int nanomina) along the canonical chain."""
    oracle = {}
    for f in files:
        d = load_block(f)["data"]
        for _idx, acct in d.get("accounts_accessed", []):
            if acct.get("token_id") != MINA_TOKEN:
                continue
            oracle[acct["public_key"]] = int(acct["balance"])
    return oracle


def indexer_balance(pk, height):
    q = ('{ stagedLedgerAccounts(query:{publicKey:"%s", blockchain_length:%d})'
         ' { balance_nano } }') % (pk, height)
    d = gql(q)
    data = d.get("data")
    if not data:
        return None  # GraphQL error (data: null) or empty
    accs = data.get("stagedLedgerAccounts")
    if not accs:
        return None
    return accs[0]["balance_nano"]


def file_height(f):
    return int(os.path.basename(f).split("-", 2)[1])


# Compare at a height buried below the moving tip, so the ledger there is
# settled (not mid-reorg during an in-progress sync).
MARGIN = 155


def main():
    hash_idx = build_hash_index()
    tip_hash, tip_h = best_tip()
    target_h = tip_h - MARGIN
    print(f"canonical tip: {tip_hash} @ {tip_h}; comparing at settled height {target_h}")
    files = canonical_chain(tip_hash, hash_idx)
    files = [f for f in files if file_height(f) <= target_h]
    tip_h = target_h  # everything below compares at the settled height
    print(f"canonical chain length up to {target_h}: {len(files)}")
    oracle = build_oracle(files)
    print(f"distinct MINA accounts in accounts_accessed: {len(oracle)}")

    ok = bad = missing = 0
    mismatches = []
    for pk, want in sorted(oracle.items()):
        got = indexer_balance(pk, tip_h)
        if got is None:
            missing += 1
            mismatches.append((pk, want, "MISSING"))
            continue
        if int(got) == want:
            ok += 1
        else:
            bad += 1
            mismatches.append((pk, want, int(got)))

    print(f"\n=== RESULT: {ok} exact / {ok+bad+missing} total"
          f" ({bad} wrong, {missing} missing) ===")
    for pk, want, got in mismatches:
        tag = " <<< HOLDOUT" if pk == HOLDOUT else ""
        dw = f"{want/1e9:.3f}"
        dg = got if isinstance(got, str) else f"{got/1e9:.3f}"
        print(f"  {pk}  want={dw}  got={dg}{tag}")

    # explicit holdout line
    if HOLDOUT in oracle:
        print(f"\nHOLDOUT oracle={oracle[HOLDOUT]/1e9:.3f} "
              f"indexer={indexer_balance(HOLDOUT, tip_h)}")
    else:
        print(f"\nHOLDOUT {HOLDOUT} not in oracle set")


if __name__ == "__main__":
    main()
