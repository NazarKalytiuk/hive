POST http://127.0.0.1:3001/test/cleanup
HTTP 200

GET http://127.0.0.1:3001/users
HTTP 401
[Asserts]
jsonpath "$.error" == "unauthorized"

POST http://127.0.0.1:3001/auth/login
Content-Type: application/json
{
  "email": "auth@example.com",
  "password": "secret"
}
HTTP 200
[Captures]
token: jsonpath "$.token"

GET http://127.0.0.1:3001/users?page=1&limit=1
Authorization: Bearer {{token}}
HTTP 200
[Asserts]
jsonpath "$.meta.page" == 1
jsonpath "$.meta.limit" == 1
