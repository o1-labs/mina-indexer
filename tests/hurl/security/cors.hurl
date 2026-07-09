#
# CORS allow-list (--web-cors-allowed-origins).
#

# An allowed origin is echoed back in Access-Control-Allow-Origin.
GET {{url}}/health
Origin: https://allowed.example
HTTP 200
[Asserts]
header "access-control-allow-origin" == "https://allowed.example"

# A disallowed origin is served (server-side) but NOT granted the CORS header,
# so a browser blocks the cross-origin script from reading the response.
GET {{url}}/health
Origin: https://evil.example
HTTP 200
[Asserts]
header "access-control-allow-origin" not exists
