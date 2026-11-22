use crate::domain::request::{Request, Response};
use reqwest::{Method, blocking::Client, redirect::Policy};
use std::error::Error as _;
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
            return "Request timed out".to_owned();
        }
        let summary = if error.is_connect() {
            "Could not connect to the server"
        } else {
            "Could not complete the HTTP request"
        };
        format!("{summary}: {}", root_cause(&error))
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

// The innermost error names the real reason (refused, DNS, certificate) and never holds the URL.
fn root_cause(error: &reqwest::Error) -> String {
    std::iter::successors(error.source(), |&error| error.source())
        .last()
        .map_or_else(|| "unknown error".to_owned(), ToString::to_string)
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

    fn request(url: &str) -> Request {
        Request {
            method: "GET".into(),
            url: url.parse().unwrap(),
            headers: Default::default(),
            body: String::new(),
        }
    }

    #[test]
    fn shows_redirects_as_responses_masks_cookies_and_explains_refused_connections() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/start?key=hidden", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 1024]).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: sid=secret\r\n\
                      Content-Length: 2\r\nConnection: close\r\n\r\nhi",
                )
                .unwrap();
        });
        let response = send(&client().unwrap(), request(&url)).unwrap();
        server.join().unwrap();
        assert_eq!((response.status, response.raw.as_str()), (302, "hi"));
        let header = |name| {
            response
                .headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(header("location"), Some("/next"));
        assert_eq!(header("set-cookie"), Some("[masked]"));

        // The listener is closed now, so the same port refuses the connection.
        let error = send(&client().unwrap(), request(&url)).err().unwrap();
        assert!(error.starts_with("Could not connect to the server: "));
        assert!(!error.contains("hidden"));
    }
}
