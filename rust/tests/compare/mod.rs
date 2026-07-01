//! Cross-checks the mina-indexer GraphQL API against an external source of
//! truth. Network-bound, so every test here is `#[ignore]`d and run on a
//! schedule (see `.github/workflows/compare.yml`), not in the normal suite.

mod devnet_archive;
