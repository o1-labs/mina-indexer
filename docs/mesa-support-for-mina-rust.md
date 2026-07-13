# Handoff: add mesa support to the Mina Rust node (o1-labs/mina-rust)

**Audience:** an agent/engineer picking up the work in the `o1-labs/mina-rust` (formerly
`openmina/openmina`) repo. You do **not** need the mina-indexer repo to do this work; it is
the consumer that motivated it.

**Status:** investigated and reproduced 2026-07-13. Not started.

---

## The problem in one sentence

`mina-tree` (the ledger crate in `o1-labs/mina-rust`, published as **`mina-tree`**, at
`crates/ledger`) **cannot represent a mesa account**, so it cannot compute a mesa ledger
Merkle root, so nothing downstream (account inclusion proofs, ledger-root checks,
`mina-verify`'s account APIs) works on mesa.

## The concrete blocker

Mesa widened the zkApp application state from **8** field elements to **32**.

```rust
// crates/ledger/src/account/account.rs:783
pub struct ZkAppAccount {
    pub app_state: [Fp; 8],   // <-- mesa needs 32
    pub verification_key: Option<VerificationKeyWire>,
    pub zkapp_version: u32,
    pub action_state: [Fp; 5],
    pub last_action_slot: Slot,
    pub proved_state: bool,
    pub zkapp_uri: ZkAppUri,
}
```

### Evidence (both independently confirmed)

1. **Live mesa-mut daemon.** Querying any zkApp account returns a `zkappState` array with
   **32** entries. (Compare: devnet returns **8**.)
2. **Mesa genesis state dump.** Every one of the 1,818 zkApp accounts in
   `/ledger/accounts` has an `app_state` of length 32 — no exceptions.

Feeding a mesa ledger into `mina-tree` fails at conversion with
`ZkAppStateTooLong`, long before you get anywhere near a hash.

Mesa is protocol **transaction version 3** (devnet/mainnet are on 2).

## Why this is not a one-line change

`app_state` arity is not a detail of one struct. It is load-bearing in at least four
places, and they must all agree or the hash silently diverges:

1. **Account layout** — `ZkAppAccount.app_state: [Fp; 8]` (`crates/ledger/src/account/account.rs`).
2. **Account hashing** — the `ToInputs` impl for `ZkAppAccount` packs `app_state` into the
   Poseidon sponge. Changing the arity changes the account hash, hence the ledger root.
   This must match Mina's OCaml **bit-for-bit**.
3. **Binprot / wire types** — `mina-p2p-messages` has generated `MinaBaseZkappAccountStableV2`
   (and friends) with the 8-wide state. Mesa implies a **new stable version** for the
   tx-v3 types, not an edit of the V2 ones (the V2 types must keep working for devnet /
   mainnet).
4. **Circuits** — `crates/ledger/src/proofs/` encodes zkApp state size as a circuit
   constant (see `proofs/constants.rs`, `proofs/zkapp*`). If you only need *hashing* and
   *account proofs* (not proving), you may be able to leave the prover paths alone — but
   you must confirm nothing you touch is shared with the verifier path.

Treat this as "implement the mesa hardfork's account model in mina-rust," not "bump a
constant."

## Recommended scoping

The full mesa hardfork in openmina is large. But the **useful subset** is much smaller:

> Make `mina-tree` able to *represent and hash* mesa accounts, and compute a mesa ledger
> Merkle root + inclusion paths.

That subset does **not** require block production, p2p, scan state, or proving. It is
plausibly: account layout + `ToInputs` + the wire types + a tx-v3 tree version.

Suggested shape (discuss upstream before building — see "Talk to upstream first"):

- Introduce a tree/tx **version** discriminant rather than mutating V2 in place. There is
  already a `TreeVersion` trait with `V1` and `V2` markers
  (`crates/ledger/src/tree_version.rs`) — note that today **only `V2` has a `BaseLedger`
  impl** (`impl BaseLedger for DatabaseImpl<V2>`), so `V1` is already a dead marker. Adding
  a `V3`/mesa version alongside `V2` is the natural extension point.
- Make `app_state` a version-parameterised type (const generic `[Fp; N]`, or an enum) so
  V2 and mesa can coexist in one binary. Consumers (like the indexer) need to serve
  multiple networks from one build.

## The acceptance test — you get a free, exact oracle

This is the good news: **you do not have to guess whether you got the hashing right.** The
protocol publishes the answer.

Build a mesa ledger from the genesis state dump, compute `merkle_root()`, and compare to
the known mesa genesis ledger hash:

```
MESA_GENESIS_LEDGER_HASH = jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT
```

If the root matches, your account layout, your `ToInputs`, and your Poseidon packing are
all correct — bit-for-bit — for 277,307 accounts including 1,818 zkApps. If it doesn't
match, something is wrong and you must not ship it. There is no partial credit and no
ambiguity. Use it as the regression test.

The equivalent oracle already passes for the **Berkeley (V2)** ledger, which proves the
approach and the harness are sound:

```
228,174 accounts -> jwNw4qb6tnNhpQNxiMLem9WumxZTwmbSx3fYXW4FP3hZRkoQJSE   (== HARDFORK_GENESIS_LEDGER_HASH)  ✅
```

### A working harness already exists — reuse it

A spike that does exactly this (load a Mina genesis state dump -> `mina-tree` ->
`merkle_root()` -> compare to expected base58) was written on 2026-07-13. It currently
**passes on the Berkeley V2 ledger** and **fails on mesa with `ZkAppStateTooLong`** — i.e.
it is already pointed at the exact bug. Ask the mina-indexer maintainer (@dkijania) for
`ledger-root-spike`; it is ~150 lines plus a vendored converter.

## Landmine: openmina's own daemon-json parser cannot read real ledgers

`crates/node/src/daemon_json/json_ledger.rs` is the daemon-json -> `mina-tree::Account`
converter. **It cannot parse either production state dump as-is.** Fixing these is
probably worth a PR on its own, independent of mesa:

| Field | openmina expects | Reality |
|---|---|---|
| `token` | `token_id`, decimal field element | key is `token`; **base58** on mesa, **decimal** on Berkeley |
| `token_symbol` | `Vec<u8>` | a JSON **string** (`""`, `"BC"`, …) |
| `zkapp_uri` | `Vec<u8>` | a JSON **string** |
| `zkapp_version` | `u32` | a **string** (`"0"`) |
| `last_action_slot` | `String` | a bare **number** on mesa |
| `set_verification_key` | `{auth, txn_version}` struct | a bare **string** (`"signature"`) on Berkeley |
| `verification_key` | `Option<()>` + **hard error** (`VerificationKeyParsingNotSupported`) | a **base64 binprot vk** on every real zkApp — and it is **hashed into the account** |

Two of these are correctness traps, not ergonomics:

- **Verification keys.** openmina literally refuses to parse them. Every real zkApp has
  one and it contributes to the account hash. Decode via
  `MinaBaseVerificationKeyWireStableV1::from_base64(..)` ->
  `VerificationKey::try_from(&wire)` -> `VerificationKeyWire::new(vk)`.
- **Legacy `set_verification_key: "signature"`** carries no `txn_version`, but the
  permission *is* hashed. Empirically the value that reproduces the Berkeley root is
  **`txn_version = 2`** (1 and 3 both produce different, wrong roots). Do not guess this —
  it is hash-sensitive.

The spike above already implements all of these fixes; lift them.

## Talk to upstream first

`o1-labs/mina-rust` is **not archived** but is **quiet — last commit 2026-02-18** (122
stars, 394 open issues). Before building:

1. Open an issue asking whether mesa / tx-v3 support is planned or in progress somewhere.
   You may be duplicating work, or there may be a design they want followed.
2. Expect a PR to sit. Plan for a pinned fork (`o1-labs/mina-rust` rev-pinning is already
   the norm — `mina-verify` pins `ab69eaed`) rather than assuming an upstream merge.

## Definition of done

- [ ] `mina-tree` can represent a mesa zkApp account (32-wide `app_state`) **without
      breaking** the V2 (devnet/mainnet) account model in the same binary.
- [ ] Mesa genesis state dump hashes to `jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT`.
- [ ] The Berkeley V2 oracle **still** passes (`jwNw4qb6tnNh…RkoQJSE`) — no regression.
- [ ] `merkle_path()` / account inclusion proofs work on a mesa ledger.
- [ ] (Ideally) `daemon_json` parses real Berkeley and mesa state dumps, verification keys
      included.
