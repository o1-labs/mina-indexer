# Blockberry endpoint coverage

Where every Blockberry (Minascan backend) endpoint is served from when the Mina
Indexer stands in for it — see issue #95. A Blockberry-shaped REST gateway sits
in front of the indexer's GraphQL; this table records, per endpoint, **what
answers it**.

## Sources

| Source | Meaning |
|---|---|
| **indexer** | Answered by one indexer GraphQL query (named below). |
| **indexer (compose)** | Gateway composes 2+ indexer queries into one response. |
| **gateway join** | Gateway joins the indexer with another service (e.g. a tip height from the light node). |
| **light node** | `mina-light-node` — data the indexer does not hold (mempool, gossip tip). |
| **off-chain** | Not chain data (price, registries, display metadata). Stub, proxy, or drop. |
| **planned** | Indexer *will* answer it, but the index isn't built yet (tracked on #95). |

Amounts from the indexer are **nanomina** unless a field is named otherwise;
lists take `limit` (≤ 1000) + `offset` and have a sibling `xxxCount` for totals
(see [graphql-pagination.md](./graphql-pagination.md)).

## Lists & entities — indexer

| Blockberry endpoint | Indexer query | Notes |
|---|---|---|
| `getAccounts` | `accounts` / `accountsCount` | sorts `BALANCE` + `NONCE` (#95 item 2); filter by pk/token/balance/zkapp |
| `getAccountStats` | `accounts` → `AccountWithMeta.pk_*` | per-pk block/snark/command counts are fields on the account |
| `getBlocks` | `blocks` / `blocksCount` | filter by height/slot/creator/canonicity |
| `getTransactions` | `transactions` / `transactionsCount` | user commands; filter by pk (`from`/`to`), block, hash |
| `getInternalCommands` | `internalCommands` / `internalCommandsCount` | coinbase / fee-transfer |
| `getSnarks` | `snarks` / `snarksCount` | filter by prover/block |
| `getSnarkers` | `topSnarkers` / `topSnarkersCount` | per-prover fee aggregates |
| `getStakes` | `stakes` / `stakesCount` | staking-ledger accounts for an epoch |
| `getEvents` / `getActions` | `events` / `actions` (+ counts) | zkApp events / actions by address+token |
| `getTopStakers` | `topStakers` / `topStakersCount` | per-epoch stake + blocks/slots produced |
| `getTokens` / `getTokenHolders` | `tokens` / `tokenHolders` (+ counts) | custom-token supply & holders |

## Validators — indexer (#95 item 6)

`topStakers` **is** the on-chain validator surface (per epoch it returns
delegation totals, delegator count, and canonical/supercharged blocks + slots
produced, filterable by `public_key`/`username`, paginated). Decision (#95 item
6): the gateway composes the validator endpoints from it rather than adding a
second name for the same concept to the GraphQL contract.

| Blockberry endpoint | Served by | Mapping |
|---|---|---|
| `getValidators` | indexer | `topStakers(query: { epoch }, sortBy, limit, offset)` + `topStakersCount` for paging |
| `getValidatorByAddress` | indexer (compose) | `topStakers(query: { epoch, public_key })` for stake/production **＋** `accounts(query: { publicKey })` for the validator's own balance/account |

Validator *metadata* (name, website, fee) is off-chain registry data — out of
scope for the indexer either way.

## Charts & rollups — indexer

| Blockberry endpoint | Indexer query | Notes |
|---|---|---|
| `getTransactionsCountChart` | `transactionsCountChart(bucket[, address])` | day/week/month; optional per-address filter (#95 item 3) |
| `getZkAppTransactionsCountChart` | `zkappTransactionsCountChart(bucket)` | network-wide only — no per-address zkApp index |
| `getDelegationAmountChart` | `delegationAmountChart(address)` | per-epoch stake delegated to an address |
| `getTimeLocksAll` / `Day` / `Month` / `Year` | `timeLocks(bucket: ALL/DAY/MONTH/YEAR)` | locked/vesting supply rollup (#95 item 4) |

## Verification-key history — planned (#95 item 5)

| Blockberry endpoint | Served by | Notes |
|---|---|---|
| `getLastVerificationKeyChange` | planned (indexer) | needs a per-zkApp VK change-log index (height, timestamp, old→new hash) — a store change + version bump + rebuild; not built yet |
| `getVerificationKeyHistory` | planned (indexer) | same index; current VK is already on `accounts.zkapp` |

## Dashboard / summary — indexer

| Blockberry endpoint | Served by | Notes |
|---|---|---|
| `getDashboardInfo` | indexer | REST `/summary` (`blockchain_summary`): total/locked/circulating supply, account & block counts, producers, snarks |

## Not indexer data

| Blockberry endpoint / need | Source | Why |
|---|---|---|
| `getPendingTransactions` | light node | No mempool in the indexer — unconfirmed txns exist only in gossip |
| `getLatestBlockStateHash` | light node | Indexer *can* answer via `blocks(sortBy: BLOCKHEIGHT_DESC, limit: 1)`, but the proof-verified tip from the light node is preferred; that fallback covers a light-node outage |
| `getBlockConfirmationByTransactionHash` | gateway join | txn block height (indexer) − chain tip height (light node) |
| Proof-backed balance / inclusion | #32 | Merkle-path query — separate feature, not duplicated here |
| MINA price, scam/security lists, display names & images | off-chain | Not chain data; stub, proxy, or drop from the contract |

## #95 status snapshot

Done: pagination + counts (item 1), sort widening (item 2), charts (item 3),
time-locks (item 4), validators mapping (item 6, this doc). Remaining: VK-change
history index (item 5).
