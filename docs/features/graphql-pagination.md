# GraphQL list pagination (`offset` + `xxxCount`)

Every list query in the GraphQL schema is paginated the same way, so a gateway
(e.g. a Blockberry-compatible REST surface, see issue #95) can serve
`page`/`size`/`totalPages` on top of it.

## The contract

Each list query `xxx(query, sort_by, limit, offset)` takes:

- `limit` (default 100) — max rows to return, **capped at
  `GRAPHQL_MAX_PAGE_SIZE` (1000)**.
- `offset` (default 0) — matching rows to skip before `limit`.

and has a sibling **`xxxCount(query[, sort_by])`** returning the total number of
rows that match the same `query`. A gateway computes:

```
page   = offset / size
totalPages = ceil(xxxCount / size)
```

Reading a set larger than 1000 is done by paging: hold `size <= 1000` and walk
`offset = 0, size, 2*size, …` until a page comes back empty.

Queries covered: `accounts`, `blocks`, `transactions`, `internalCommands`,
`snarks`, `stakes`, `events`, `actions`, `topStakers`, `topSnarkers`, `tokens`,
`tokenHolders`. (`stagedLedgerAccounts` also takes `offset`, but it is a
point-in-time ledger *snapshot*, not a Blockberry-style filtered list, so it has
no separate count — its total is the ledger size.)

## The list and the count cannot drift

`xxx` and `xxxCount` always apply the **same** row filter:

- Simple resolvers share the filter predicate directly, and the count avoids
  building the per-row response object where possible (it only needs the stored
  record + the filter).
- Multi-branch resolvers (`internalCommands`, `snarks`, `transactions`, `blocks`)
  route through a single shared `xxx_dispatch(db, query, sort_by, limit, offset)`.
  The list calls it capped + paged; the count calls the **same** dispatch
  uncapped/unpaged and returns the length. Same code path ⇒ the page and its
  total can never disagree for a given database state.

## Semantics over a moving chain tip

Pagination is **best-effort against the current best tip**, not a snapshot:

- A single query resolves against the database as it is at that moment.
- Two calls a gateway makes for one logical page — `xxx(offset, limit)` and
  `xxxCount(query)` — are **separate** queries. If a block is added or a reorg
  happens between them, the count and the page can reflect slightly different
  chain states (a total that ticks up, or a row that shifts canonicity).
- This is inherent to paging a live chain and matches the behaviour of the API
  we are replacing (Blockberry has the same property). Clients that need a stable
  view should pin a `blockHeight`/`state_hash` filter where the query supports it
  (e.g. `blocks`, `transactions`), which resolves against that fixed point rather
  than the moving tip.

For a strongly-consistent point-in-time ledger, use `stagedLedgerAccounts` with a
`state_hash` / `ledger_hash` / `blockchain_length`, which reconstructs the ledger
at exactly that block.
