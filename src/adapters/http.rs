use crate::domain::request::{Request, Response};
use reqwest::{Method, blocking::Client, redirect::Policy};
use std::io::Read;
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const PREVIEW_LIMIT: usize = 256 * 1024;

pub fn client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .build()
}

pub fn send(client: &Client, request: Request) -> Result<Response, String> {
    let method =
        Method::from_bytes(request.method.as_bytes()).map_err(|_| "Invalid HTTP method")?;
    let started = Instant::now();
    let builder = client
        .request(method, request.url)
        .headers(request.headers)
        .timeout(REQUEST_TIMEOUT);
    let response = if request.body.is_empty() {
        builder
    } else {
        builder.body(request.body)
    }
    .send()
    .map_err(|error| {
        if error.is_timeout() {
            "Request timed out"
        } else if error.is_connect() {
            "Could not connect to the server"
        } else {
            "Could not complete the HTTP request"
        }
    })?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(key, value)| {
            let displayed = match key.as_str() {
                "set-cookie" | "cookie" | "authorization" | "proxy-authorization" => "[masked]",
                _ => value.to_str().unwrap_or("[non-text header]"),
            };
            (key.to_string(), displayed.to_owned())
        })
        .collect();
    let mut bytes = Vec::new();
    response
        .take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|source| source.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout)
            {
                "Response body timed out"
            } else {
                "Could not read the response body"
            }
        })?;
    let elapsed_ms = started.elapsed().as_millis();
    Ok(preview(status, elapsed_ms, headers, &bytes))
}

fn preview(
    status: u16,
    elapsed_ms: u128,
    headers: Vec<(String, String)>,
    bytes: &[u8],
) -> Response {
    let truncated = bytes.len() > PREVIEW_LIMIT;
    let bytes = &bytes[..bytes.len().min(PREVIEW_LIMIT)];
    let raw = String::from_utf8_lossy(bytes).into_owned();
    let pretty = (!truncated)
        .then(|| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .flatten()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .filter(|pretty| pretty.len() <= PREVIEW_LIMIT && pretty != &raw);
    Response {
        status,
        elapsed_ms,
        headers,
        raw,
        pretty,
        bytes: bytes.len(),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_json_and_limits_response_memory() {
        assert_eq!(
            preview(200, 1, vec![], br#"{"ok":true}"#).pretty.as_deref(),
            Some("{\n  \"ok\": true\n}")
        );
        let response = preview(200, 1, vec![], &vec![b'x'; PREVIEW_LIMIT + 1]);
        assert!(response.truncated);
        assert_eq!(response.raw.len(), PREVIEW_LIMIT);
        assert!(response.pretty.is_none());
    }
}
