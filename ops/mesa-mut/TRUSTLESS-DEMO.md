# Trustless mina-indexer — demo runbook (mesa-mut)

**Claim:** the indexer ingests a block only after an independent service has verified
the block's Pickles/kimchi SNARK proof. A valid proof attests the entire chain back to
(fork) genesis by recursion, so the indexer trusts *math*, not whoever served the block.

Proven end-to-end on **2026-06-15** against live mesa-mut (caught up to network tip
**301728**, 0 false rejections; a tampered-proof block is rejected).

---

## Architecture

```
        precomputed blocks (GCS: mesa-mut-precomputed-blocks)
                       │  /bin/mesa-pull (fetch + recovery)
                       ▼
   ┌─────────────────────────────────────┐        POST /verify (block JSON)
   │  mina-indexer  (mina-indexer:mesa-prod)│ ───────────────────────────────► ┌──────────────────────────┐
   │                                       │                                   │  mina-verifier            │
   │  process_event / reconcile_blocks_dir │ ◄─────────────────────────────── │  (mina-verify:mesa-ab84160)│
   │     │  --verify-block-exe             │     {"valid":true,...} → ingest   │  checks SNARK proof w/    │
   │     ▼  /bin/verify-block <net> <file> │     {"valid":false}  → REJECT     │  mesa VK (fork-297734)    │
   │  ingest only if exit 0                │     unreachable/4xx  → REJECT     │  GET /health  POST /verify │
   └─────────────────────────────────────┘        (fail-closed)               └──────────────────────────┘
                       │                                          docker network: trustless
                       ▼  RocksDB at /data/db
```

- **Gate location (code):** `rust/src/server.rs` — `process_event` (~L562, fs-watcher path)
  and `reconcile_blocks_dir` (~L675, 180s safety-net path). Both call
  `verify_block(exe, network, path)` and skip ingestion on non-zero exit.
- **Shim:** `ops/mesa-mut/verify-block.sh`, baked into the indexer image as `/bin/verify-block`
  (flake `verify-block` package). Called as `verify-block <network> <block-file>`; POSTs the
  block to `$VERIFY_ENDPOINT` and exits 0 iff the response contains `"valid":true`.
  **Fail-closed:** verifier unreachable → exit 2; proof invalid → exit 1.
- **Sidecar API:** `GET /health` → `{"status":"ok","network":...}`; `POST /verify` (body =
  precomputed-block JSON) → `200 {"valid":true,height,state_hash,...}` /
  `200 {"valid":false,"error":"block proof did not verify"}` / `400` on undecodable.
  Reads the body lossily (mesa blocks carry raw bytes in `sok_digest`).
- **Backlog is not re-verified:** `reconcile_blocks_dir` only considers heights ≥ `tip−K`
  and skips any block already in the witness tree *before* the gate — so flipping verify on
  against an existing DB gates new blocks only.

---

## Build the pieces (once, on this host)

```bash
# 1) Sidecar binary — from release/mesa (proof-systems ab84160)
cd /home/darek/work/minaprotocol/mina-verify-ab84160
cargo build --release -p mina-verify-server          # ~3m

# 2) Thin sidecar image with the mesa VK baked in
CTX=/tmp/verify-sidecar-img; mkdir -p "$CTX"
cp target/release/mina-verify-server "$CTX"/
cp /home/darek/work/minaprotocol/mesa_vk_297734.json "$CTX"/mesa_vk.json   # md5 26dc191a, fork-297734
cat > "$CTX"/Dockerfile <<'EOF'
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY mina-verify-server /usr/local/bin/mina-verify-server
COPY mesa_vk.json /vk/mesa_vk.json
ENV MINA_VK_JSON=/vk/mesa_vk.json BIND=0.0.0.0:8090
EXPOSE 8090
ENTRYPOINT ["mina-verify-server"]
EOF
docker build -t mina-verify:mesa-ab84160 "$CTX"

# 3) Indexer image mina-indexer:mesa-prod — already built from feat/trustless-verify
#    (bakes /bin/verify-block via the flake verify-block package)
```

## Bring it up

```bash
docker compose -f ops/mesa-mut/docker-compose.trustless.yml up -d
# (note: the production /data dirs must be writable by the container uid 65534)
```

---

## Demo script (talking points + live commands)

**1. Show the gate is wired and the verifier is live.**
```bash
docker inspect mina-indexer --format '{{json .Args}}' | tr ',' '\n' | grep verify-block   # --verify-block-exe /bin/verify-block
docker run --rm --network trustless curlimages/curl -fsS http://mina-verifier:8090/health  # {"status":"ok","network":"custom"}
```

**2. Verify a real block by hand — the proof checks out (`valid:true`).**
```bash
cd /home/darek/work/o1labs/mina-indexer/data/blocks
B=$(ls -t mesa-*.json | head -1)
docker run --rm --network trustless -v "$PWD":/b:ro curlimages/curl -fsS \
  -H 'Content-Type: application/json' --data-binary @/b/"$B" http://mina-verifier:8090/verify
# → {"valid":true,"height":...,"state_hash":...}
```

**3. The indexer ingests live blocks only through the gate.**
```bash
docker logs mina-indexer 2>&1 | grep -E "Reconciled on-disk block|Added block" | tail -5
docker logs mina-indexer 2>&1 | grep -c "Rejected block"     # 0 — every legit block passed
```

**4. The money shot — tamper a proof, watch the indexer REFUSE it.**
```bash
cd /home/darek/work/o1labs/mina-indexer/data/blocks
SRC=$(ls mesa-301900-*.json 2>/dev/null | head -1)   # any height the indexer hasn't ingested yet
# byte-flip one char deep inside the base64 proof (keeps the rest of the file intact)
python3 - "$SRC" <<'PY'
import sys; f=sys.argv[1]
d=bytearray(open(f,'rb').read()); k=d.find(b'"protocol_state_proof":"'); p=k+len(b'"protocol_state_proof":"')+7000
d[p]=ord('A') if d[p]!=ord('A') else ord('B'); open('/tmp/'+f,'wb').write(d)
print('tampered', f)
PY
# place it where the watcher sees it, then:
docker logs -f mina-indexer 2>&1 | grep -m1 "Rejected block (proof did not verify)"
# → WARN Rejected block (proof did not verify): ".../mesa-301900-...json"
```
*(Captured this session against the identical stack — bad-proof block `mesa-297900` produced
`{"error":"block proof did not verify","valid":false}` and was kept out of the store.)*

**5. Fail-closed.** Stop the verifier and show new blocks stop being ingested:
```bash
docker stop mina-verifier
# indexer logs: "verify-block: verifier unreachable ... exit 2" → block not ingested
docker start mina-verifier
```

---

## Evidence captured 2026-06-15

| Check | Result |
|---|---|
| Sidecar build (release/mesa, ab84160) | clean, 3m13s |
| Real mesa block → `/verify` | `valid:true` (~0.3–0.9s warm) |
| Tampered-proof block → `/verify` | `valid:false` "block proof did not verify" |
| Malformed JSON / verifier down → shim | exit 2 (fail-closed) |
| In-container bad-proof block (297900) | indexer logged "Rejected block", not ingested |
| Live mesa-mut catch-up via gate | reached network tip 301728, **0** false rejections |

## Operational note
The mesa VK is **fork-pinned** to 297734. mesa-mut is a preflight network that has
re-forked before; if it re-forks, the VK changes — regenerate `mesa_vk_297734.json` from the
deployed daemon image's `config_*.json` via `print_blockchain_snark_vk` and rebuild the sidecar
image. `MINA_VK_JSON` is the single deployment input that must track the live fork point.
