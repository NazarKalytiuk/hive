GET http://127.0.0.1:3001/health
HTTP 200
[Asserts]
jsonpath "$.missing" == "x"
