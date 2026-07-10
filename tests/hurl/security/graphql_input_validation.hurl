#
# GraphQL input validation. Malformed identifiers (public key, state hash, txn
# hash, token address) are rejected with a GraphQL error rather than panicking
# inside the store decoder — the resolvers validate before querying.
#

POST {{url}}/graphql
```graphql
{ accounts(query: { publicKey: "notavalidkey" }) { publicKey } }
```
HTTP 200
[Asserts]
jsonpath "$.data" == null
jsonpath "$.errors[0].message" == "Invalid public key: notavalidkey"

POST {{url}}/graphql
```graphql
{ blocks(query: { stateHash: "notavalidhash" }, limit: 1) { stateHash } }
```
HTTP 200
[Asserts]
jsonpath "$.errors[0].message" == "Invalid state hash: notavalidhash"

POST {{url}}/graphql
```graphql
{ transactions(query: { hash: "notavalidhash" }, limit: 1) { hash } }
```
HTTP 200
[Asserts]
jsonpath "$.errors[0].message" == "Invalid txn hash: notavalidhash"
