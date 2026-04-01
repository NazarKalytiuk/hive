POST https://jsonplaceholder.typicode.com/posts
Content-Type: application/json
{
  "title": "tarn-vs-hurl",
  "body": "deep research",
  "userId": 42
}
HTTP 201
[Asserts]
header "content-type" contains "application/json"
jsonpath "$.title" == "tarn-vs-hurl"
jsonpath "$.body" == "deep research"
jsonpath "$.userId" == 42
