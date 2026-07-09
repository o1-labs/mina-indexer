#
# GraphQL introspection toggle (--graphql-disable-introspection).
# With introspection disabled, `__schema` resolves to null instead of exposing
# the schema (no error — the field is simply not populated).
#

POST {{url}}/graphql
```graphql
{
  __schema {
    queryType {
      name
    }
  }
}
```
HTTP 200
[Asserts]
jsonpath "$.data.__schema" == null
