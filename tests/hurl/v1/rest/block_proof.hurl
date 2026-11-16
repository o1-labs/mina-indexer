#
# Tier-1 verifiable block: GET /blocks/{state_hash}/proof returns the block (for
# the client to re-verify) plus canonicity material. See docs/trustless-responses.md.
#

# A canonical block (genesis is always canonical) → full proof envelope.
GET {{url}}/blocks/3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ/proof
HTTP 200
[Asserts]
jsonpath "$.state_hash" == "3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ"
jsonpath "$.height" == 1
jsonpath "$.block" exists
jsonpath "$.canonicity.target_height" == 1
jsonpath "$.canonicity.best_tip_height" exists
jsonpath "$.canonicity.anchor_state_hash" exists
jsonpath "$.canonicity.parent_chain[0]" == "3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ"

# An unknown state hash → 404 (never a fabricated proof).
GET {{url}}/blocks/3NAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/proof
HTTP 404
