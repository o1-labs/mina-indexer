#
# GraphQL query-complexity guard (--graphql-max-complexity).
# Complexity counts selected fields (aliases count separately), so 25 aliased
# scalar selections exceed a limit of 20 and the query is rejected at validation.
#

POST {{url}}/graphql
```graphql
{
  blocks(limit: 1) {
    a0: blockHeight
    a1: blockHeight
    a2: blockHeight
    a3: blockHeight
    a4: blockHeight
    a5: blockHeight
    a6: blockHeight
    a7: blockHeight
    a8: blockHeight
    a9: blockHeight
    a10: blockHeight
    a11: blockHeight
    a12: blockHeight
    a13: blockHeight
    a14: blockHeight
    a15: blockHeight
    a16: blockHeight
    a17: blockHeight
    a18: blockHeight
    a19: blockHeight
    a20: blockHeight
    a21: blockHeight
    a22: blockHeight
    a23: blockHeight
    a24: blockHeight
  }
}
```
HTTP 200
[Asserts]
jsonpath "$.data" == null
jsonpath "$.errors[0].message" == "Query is too complex."
