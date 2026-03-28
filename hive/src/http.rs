use crate::error::HiveError;
use std::collections::HashMap;
use std::time::Instant;

/// Response from an HTTP request.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
    pub duration_ms: u64,
}

/// Execute an HTTP request and return the response.
pub fn execute_request(
    method: &str,
    url: &str,
    headers: &HashMap<String, String>,
    body: Option<&serde_json::Value>,
    timeout_ms: Option<u64>,
) -> Result<HttpResponse, HiveError> {
    let client = reqwest::blocking::Client::new();

    let mut builder = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        "HEAD" => client.head(url),
        "OPTIONS" => client.request(reqwest::Method::OPTIONS, url),
        other => {
            return Err(HiveError::Http(format!(
                "Unsupported HTTP method: {}",
                other
            )));
        }
    };

    // Apply timeout
    if let Some(ms) = timeout_ms {
        builder = builder.timeout(std::time::Duration::from_millis(ms));
    }

    // Apply headers
    for (key, value) in headers {
        builder = builder.header(key, value);
    }

    // Apply body
    if let Some(body_value) = body {
        builder = builder.json(body_value);
    }

    let start = Instant::now();
    let response = builder
        .send()
        .map_err(|e| HiveError::Http(format!("Request to {} failed: {}", url, e)))?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let status = response.status().as_u16();

    let mut response_headers = HashMap::new();
    for (name, value) in response.headers() {
        if let Ok(v) = value.to_str() {
            response_headers.insert(name.as_str().to_string(), v.to_string());
        }
    }

    let body_text = response
        .text()
        .map_err(|e| HiveError::Http(format!("Failed to read response body: {}", e)))?;

    let body: serde_json::Value = if body_text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&body_text).unwrap_or(serde_json::Value::String(body_text))
    };

    Ok(HttpResponse {
        status,
        headers: response_headers,
        body,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_method_returns_error() {
        let result = execute_request("FOOBAR", "http://localhost:1", &HashMap::new(), None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported HTTP method"));
    }

    #[test]
    fn connection_refused_returns_error() {
        let result = execute_request(
            "GET",
            "http://127.0.0.1:1",
            &HashMap::new(),
            None,
            Some(1000),
        );
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HiveError::Http(_)));
    }
}
