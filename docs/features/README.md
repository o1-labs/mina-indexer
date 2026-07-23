# Novel features

The [Mina Indexer](https://github.com/Granola-Team/mina-indexer/tree/main)
and [MinaSearch](https://minasearch.com) take a different approach to several
aspects of indexing the Mina blockchain. We

- handle all [transactions](./transactions_applied_failed.md)
- paginate every list query the same way ([offset + count](./graphql-pagination.md))
- back a Blockberry-shaped API — see the [endpoint coverage table](./blockberry-endpoint-coverage.md)
