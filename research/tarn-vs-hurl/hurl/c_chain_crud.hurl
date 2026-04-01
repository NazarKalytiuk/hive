POST http://127.0.0.1:3001/test/cleanup
HTTP 200

POST http://127.0.0.1:3001/auth/login
Content-Type: application/json
{
  "email": "research@example.com",
  "password": "secret"
}
HTTP 200
[Captures]
token: jsonpath "$.token"
[Asserts]
jsonpath "$.token" matches /^[0-9a-f-]{36}$/

POST http://127.0.0.1:3001/users
Authorization: Bearer {{token}}
Content-Type: application/json
{
  "name": "Research User",
  "email": "research.user@example.com",
  "role": "editor",
  "tags": ["alpha", "beta"]
}
HTTP 201
[Captures]
userId: jsonpath "$.id"
[Asserts]
jsonpath "$.id" matches /^usr_[0-9a-f]+$/
jsonpath "$.name" == "Research User"

GET http://127.0.0.1:3001/users/{{userId}}
Authorization: Bearer {{token}}
HTTP 200
[Asserts]
jsonpath "$.email" == "research.user@example.com"
jsonpath "$.role" == "editor"

DELETE http://127.0.0.1:3001/users/{{userId}}
Authorization: Bearer {{token}}
HTTP 204
