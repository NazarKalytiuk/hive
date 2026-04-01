POST http://127.0.0.1:3001/test/cleanup
HTTP 200

POST http://127.0.0.1:3001/auth/login
Content-Type: application/json
{
  "email": "assertions@example.com",
  "password": "secret"
}
HTTP 200
[Captures]
token: jsonpath "$.token"

POST http://127.0.0.1:3001/users
Authorization: Bearer {{token}}
Content-Type: application/json
{
  "name": "Assertion User",
  "email": "assertion.user@example.com",
  "role": "viewer",
  "tags": ["alpha", "beta"]
}
HTTP 201

GET http://127.0.0.1:3001/users?page=1&limit=10
Authorization: Bearer {{token}}
HTTP 200
[Asserts]
jsonpath "$.data[0].id" matches /^usr_[0-9a-f]+$/
jsonpath "$.data[0].name" isString
jsonpath "$.data[0].name" startsWith "Assertion"
jsonpath "$.data[0].email" matches /^[^@]+@[^@]+\.[^@]+$/
jsonpath "$.data[0].tags" isList
jsonpath "$.data[0].tags" count == 2
jsonpath "$.meta.totalCount" > 0
jsonpath "$.meta.page" == 1
