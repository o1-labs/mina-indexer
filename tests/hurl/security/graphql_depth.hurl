#
# GraphQL query-depth guard (--graphql-max-depth).
# A query nested deeper than the limit is rejected at validation, before any
# resolver runs. (blocks -> transactions -> block -> stateHash = depth 4.)
#

POST {{url}}/graphql
```graphql
{
  blocks(limit: 1) {
    transactions {
      block {
        stateHash
      }
    }
  }
}
```
HTTP 200
[Asserts]
jsonpath "$.data" == null
jsonpath "$.errors[0].message" == "Query is nested too deep."
