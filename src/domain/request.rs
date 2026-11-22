use super::spec::{AuthKind, AuthOption, Encoding, Location, Operation, Parameter};
use super::variables::{VALUE_LIMIT, Variables};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http::{HeaderMap, HeaderName, HeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

const URL_LIMIT: usize = 64 * 1024;
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Server,
    Parameter(usize),
    Query,
    Headers,
    Media,
    Body,
    Auth,
    Secret,
}

#[derive(Clone)]
pub struct Draft {
    pub server: String,
    pub parameters: Vec<String>,
    pub query: String,
    pub headers: String,
    pub media: usize,
    pub body: String,
    pub auth: usize,
    pub secret: String,
}

impl Draft {
    pub fn new(operation: &Operation) -> Self {
        Self {
            server: operation.servers.first().cloned().unwrap_or_default(),
            parameters: operation
                .parameters
                .iter()
                .map(|parameter| parameter.initial.clone())
                .collect(),
            query: String::new(),
            headers: String::new(),
            media: 0,
            body: operation
                .bodies
                .first()
                .map_or_else(String::new, |body| body.example.clone()),
            auth: 0,
            secret: String::new(),
        }
    }

    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::Server => &self.server,
            Field::Parameter(index) => &self.parameters[index],
            Field::Query => &self.query,
            Field::Headers => &self.headers,
            Field::Body => &self.body,
            Field::Secret => &self.secret,
            Field::Media | Field::Auth => "",
        }
    }

    pub fn edit(self, field: Field, value: String) -> Self {
        match field {
            Field::Server => Self {
                server: value,
                ..self
            },
            Field::Parameter(index) => Self {
                parameters: self
                    .parameters
                    .into_iter()
                    .enumerate()
                    .map(|(current, previous)| {
                        if current == index {
                            value.clone()
                        } else {
                            previous
                        }
                    })
                    .collect(),
                ..self
            },
            Field::Query => Self {
                query: value,
                ..self
            },
            Field::Headers => Self {
                headers: value,
                ..self
            },
            Field::Body => Self {
                body: value,
                ..self
            },
            Field::Secret => Self {
                secret: value,
                ..self
            },
            Field::Media | Field::Auth => self,
        }
    }

    pub fn cycle(self, operation: &Operation, field: Field) -> Self {
        match field {
            Field::Server if !operation.servers.is_empty() => {
                let next = operation
                    .servers
                    .iter()
                    .position(|server| server == &self.server)
                    .map_or(0, |index| (index + 1) % operation.servers.len());
                self.edit(field, operation.servers[next].clone())
            }
            Field::Media => {
                let media = (self.media + 1) % (operation.bodies.len() + 1);
                let example = |index| {
                    operation
                        .bodies
                        .get(index)
                        .map_or("", |body: &super::spec::Body| body.example.as_str())
                };
                let body = if self.body == example(self.media) {
                    example(media).to_owned()
                } else {
                    self.body
                };
                Self {
                    media,
                    body,
                    ..self
                }
            }
            Field::Auth => Self {
                auth: (self.auth + 1) % (operation.auth.len() + 1),
                secret: String::new(),
                ..self
            },
            _ => self,
        }
    }

    pub fn auth_option<'a>(&self, operation: &'a Operation) -> Option<&'a AuthOption> {
        self.auth
            .checked_sub(1)
            .and_then(|index| operation.auth.get(index))
    }

    fn parameters<'a>(
        &'a self,
        operation: &'a Operation,
    ) -> impl Iterator<Item = (&'a Parameter, &'a str)> + Clone + 'a {
        operation
            .parameters
            .iter()
            .zip(self.parameters.iter().map(String::as_str))
            .filter(|(_, value)| !value.is_empty())
    }

    pub fn has_query(&self, operation: &Operation) -> bool {
        !self.query.trim().is_empty()
            || self
                .parameters(operation)
                .any(|(param, _)| param.location == Location::Query)
            || self
                .auth_option(operation)
                .is_some_and(|option| option.kind == AuthKind::ApiKey(Location::Query))
    }
}

pub struct Request {
    pub method: String,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: String,
}

pub struct Response {
    pub status: u16,
    pub elapsed_ms: u128,
    pub headers: Vec<(String, String)>,
    pub raw: String,
    pub pretty: Option<String>,
    pub bytes: usize,
    pub truncated: bool,
}

pub fn documented(operation: &Operation, status: u16) -> bool {
    let exact = status.to_string();
    let range = format!("{}XX", status / 100);
    operation
        .statuses
        .iter()
        .any(|key| matches!(key.as_str(), "default") || key == &exact || key == &range)
}

pub fn destination(
    operation: &Operation,
    draft: &Draft,
    variables: &Variables,
) -> Result<Url, String> {
    let server = variables.expand(&draft.server)?;
    if server.trim().chars().any(char::is_control) {
        return Err("Server URL contains a control character".into());
    }
    let server = Url::parse(server.trim()).map_err(|_| "Enter an absolute HTTP(S) server URL")?;
    if !matches!(server.scheme(), "http" | "https")
        || server.host().is_none()
        || !server.username().is_empty()
        || server.password().is_some()
        || server.query().is_some()
        || server.fragment().is_some()
    {
        return Err(
            "Server URL must use HTTP(S) and contain no credentials, query, or fragment".into(),
        );
    }
    let path = operation.path.split('/').skip(1).try_fold(
        String::new(),
        |mut path, segment| -> Result<_, String> {
            let segment = expand_segment(operation, draft, variables, segment)?;
            if path.len() + segment.len() + 1 > URL_LIMIT {
                return Err("Request URL exceeds 64 KiB".into());
            }
            path.push('/');
            path.push_str(&segment);
            Ok(path)
        },
    )?;
    let mut url = server;
    url.set_path(&format!("{}{}", url.path().trim_end_matches('/'), path));
    if url.as_str().len() > URL_LIMIT {
        return Err("Request URL exceeds 64 KiB".into());
    }
    Ok(url)
}

fn expand_segment(
    operation: &Operation,
    draft: &Draft,
    variables: &Variables,
    segment: &str,
) -> Result<String, String> {
    let expanded = segment.split_inclusive('}').try_fold(
        String::new(),
        |mut expanded, part| -> Result<_, String> {
            let Some((literal, key)) = part.split_once('{') else {
                if part.contains('}') {
                    return Err("Unexpected closing brace in path template".into());
                }
                expanded.push_str(part);
                return Ok(expanded);
            };
            let key = key.strip_suffix('}').ok_or("Unclosed path parameter")?;
            let (parameter, value) = draft
                .parameters(operation)
                .find(|(param, _)| param.location == Location::Path && param.name == key)
                .ok_or_else(|| format!("Fill in path parameter {key}"))?;
            let expanded_value = variables.expand(value)?;
            if expanded_value.is_empty() {
                return Err(format!("Fill in path parameter {key}"));
            }
            let value = match parameter.encoding {
                Encoding::Manual if !expanded_value.contains('/') => {
                    std::borrow::Cow::Borrowed(expanded_value.as_ref())
                }
                Encoding::Manual => {
                    return Err(format!("Encode '/' as %2F in path parameter {key}"));
                }
                _ => utf8_percent_encode(&expanded_value, COMPONENT).into(),
            };
            if expanded.len() + literal.len() + value.len() > URL_LIMIT {
                return Err("Request URL exceeds 64 KiB".into());
            }
            expanded.push_str(literal);
            expanded.push_str(&value);
            Ok(expanded)
        },
    )?;
    let decoded = percent_encoding::percent_decode_str(&expanded).decode_utf8_lossy();
    if matches!(decoded.as_ref(), "." | "..")
        || expanded.contains(['?', '#', '\\'])
        || expanded.chars().any(char::is_control)
    {
        return Err("Path contains an invalid or relative segment".into());
    }
    Ok(expanded)
}

fn pairs(
    source: &str,
    label: &str,
    variables: &Variables,
) -> Result<Vec<(String, String)>, String> {
    const PAIR_LIMIT: usize = 256;
    let pairs = source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(PAIR_LIMIT + 1)
        .map(|(index, line)| {
            let (key, value) = line
                .split_once(':')
                .filter(|(key, _)| !key.trim().is_empty())
                .ok_or_else(|| format!("{label}, line {}: use name: value", index + 1))?;
            Ok((key.trim().to_owned(), value.trim()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if pairs.len() > PAIR_LIMIT {
        return Err(format!("{label} supports at most {PAIR_LIMIT} rows"));
    }
    pairs
        .into_iter()
        .try_fold((Vec::new(), 0), |(mut rows, bytes), (key, value)| {
            let value = variables.expand(value)?;
            let bytes = add_expanded_bytes(bytes, &value)?;
            rows.push((key, value.into_owned()));
            Ok((rows, bytes))
        })
        .map(|(rows, _)| rows)
}

fn add_expanded_bytes(bytes: usize, value: &str) -> Result<usize, String> {
    let bytes = bytes + value.len();
    if bytes > VALUE_LIMIT {
        return Err("Expanded request fields exceed 1 MiB".into());
    }
    Ok(bytes)
}

fn header((key, value): (String, String)) -> Result<(HeaderName, HeaderValue), String> {
    let name = HeaderName::from_bytes(key.as_bytes()).map_err(|_| "Invalid header name")?;
    if matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "upgrade"
            | "proxy-authorization"
            | "trailer"
            | "te"
    ) {
        return Err(format!("The HTTP client manages the {name} header"));
    }
    let value =
        HeaderValue::from_str(&value).map_err(|_| format!("Invalid value for header {name}"))?;
    Ok((name, value))
}

fn cookie(key: &str, value: &str) -> String {
    format!(
        "{}={}",
        utf8_percent_encode(key, COMPONENT),
        utf8_percent_encode(value, COMPONENT)
    )
}

fn encode_query(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn append_query(mut query: String, key: &str, escaped: &str) -> Result<String, String> {
    let key = encode_query(key);
    if query.len() + key.len() + escaped.len() + 2 > URL_LIMIT {
        return Err("Request URL exceeds 64 KiB".into());
    }
    if !query.is_empty() {
        query.push('&');
    }
    query.push_str(&key);
    query.push('=');
    query.push_str(escaped);
    Ok(query)
}

fn query_field(query: String, parameter: &Parameter, value: &str) -> Result<String, String> {
    // shortcut: normal URL encoding for query values, including when allowReserved is declared
    let Encoding::QueryArray { explode } = parameter.encoding else {
        return append_query(query, &parameter.name, &encode_query(value));
    };
    let invalid_array = || {
        format!(
            "Enter a JSON array of scalar values for query parameter {}",
            parameter.name
        )
    };
    let items: Vec<serde_json::Value> = serde_json::from_str(value).map_err(|_| invalid_array())?;
    let mut values = items.into_iter().map(|item| match item {
        serde_json::Value::String(value) => Ok(encode_query(&value)),
        item if item.is_boolean() || item.is_number() => Ok(encode_query(&item.to_string())),
        _ => Err(invalid_array()),
    });
    if explode {
        return values.try_fold(query, |query, value| {
            append_query(query, &parameter.name, &value?)
        });
    }
    let value = values.collect::<Result<Vec<_>, _>>()?.join(",");
    append_query(query, &parameter.name, &value)
}

pub fn prepare(
    operation: &Operation,
    draft: &Draft,
    variables: &Variables,
) -> Result<Request, String> {
    let mut url = destination(operation, draft, variables)?;
    let auth = draft.auth_option(operation);
    let secret = if auth.is_some() {
        variables.expand(&draft.secret)?
    } else {
        "".into()
    };
    let auth_key = auth.and_then(|option| match option.kind {
        AuthKind::ApiKey(location) => Some((location, option.key.as_str(), secret.as_ref())),
        _ => None,
    });
    if auth.is_some() && secret.is_empty() {
        return Err("Enter credentials for the selected authentication scheme".into());
    }
    let (parameters, _) = draft
        .parameters(operation)
        .filter(|(parameter, _)| {
            parameter.location != Location::Path
                && !auth_key.is_some_and(|(location, key, _)| {
                    parameter.location == location && parameter.name == key
                })
        })
        .try_fold(
            (Vec::new(), 0),
            |(mut fields, bytes), (parameter, value)| -> Result<_, String> {
                let value = variables.expand(value)?;
                let bytes = add_expanded_bytes(bytes, &value)?;
                if !value.is_empty() {
                    fields.push((parameter, value));
                }
                Ok((fields, bytes))
            },
        )?;
    let parameters = parameters
        .iter()
        .map(|(parameter, value)| (*parameter, value.as_ref()));
    let query = parameters
        .clone()
        .filter(|(parameter, _)| parameter.location == Location::Query)
        .try_fold(String::new(), |query, (parameter, value)| {
            query_field(query, parameter, value)
        })?;
    let query_auth = auth_key.filter(|(location, _, _)| *location == Location::Query);
    let query = pairs(&draft.query, "Extra query", variables)?
        .into_iter()
        .filter(|(key, _)| !query_auth.is_some_and(|(_, name, _)| key == name))
        .chain(query_auth.map(|(_, key, value)| (key.to_owned(), value.to_owned())))
        .try_fold(query, |query, (key, value)| {
            append_query(query, &key, &encode_query(&value))
        })?;
    if url.as_str().len() + query.len() + usize::from(!query.is_empty()) > URL_LIMIT {
        return Err("Request URL exceeds 64 KiB".into());
    }
    if !query.is_empty() {
        url.set_query(Some(&query));
    }
    let fields =
        parameters.map(|(parameter, value)| (parameter.location, parameter.name.as_str(), value));

    let headers = pairs(&draft.headers, "Headers", variables)?;
    let mut headers: HeaderMap = fields
        .clone()
        .filter(|(location, _, _)| *location == Location::Header)
        .map(|(_, key, value)| (key.to_owned(), value.to_owned()))
        .chain(headers)
        .map(header)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect();
    let cookies = fields
        .chain(auth_key)
        .filter(|(location, _, _)| *location == Location::Cookie)
        .map(|(_, key, value)| cookie(key, value))
        .collect::<Vec<_>>()
        .join("; ");
    if !cookies.is_empty() {
        headers.append(
            http::header::COOKIE,
            HeaderValue::from_str(&cookies).map_err(|_| "Invalid cookie")?,
        );
    }
    if let Some(auth) = auth {
        let authorization = match auth.kind {
            AuthKind::Basic => {
                if !secret.contains(':') {
                    return Err("Basic authentication uses username:password".into());
                }
                Some(format!("Basic {}", STANDARD.encode(secret.as_bytes())))
            }
            AuthKind::Bearer => Some(format!("Bearer {secret}")),
            AuthKind::ApiKey(_) => None,
        };
        let auth_header = authorization
            .map(|value| ("Authorization".to_owned(), value))
            .or_else(|| {
                auth_key
                    .filter(|(location, _, _)| *location == Location::Header)
                    .map(|(_, key, value)| (key.to_owned(), value.to_owned()))
            });
        if let Some(pair) = auth_header {
            let (name, mut value) = header(pair)?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
    }
    let payload = variables.expand(&draft.body)?;
    if let Some(body) = operation.bodies.get(draft.media)
        && !headers.contains_key(http::header::CONTENT_TYPE)
        && (!payload.is_empty() || body.required)
    {
        let (name, value) = header(("Content-Type".into(), body.media_type.clone()))?;
        headers.insert(name, value);
    }
    Ok(Request {
        method: operation.method.clone(),
        url,
        headers,
        body: payload.into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::spec;

    fn prepare(operation: &Operation, draft: &Draft) -> Result<Request, String> {
        super::prepare(operation, draft, &Variables::default())
    }

    fn destination(operation: &Operation, draft: &Draft) -> Result<Url, String> {
        super::destination(operation, draft, &Variables::default())
    }

    #[test]
    fn builds_request_and_masks_auth_in_destination() {
        let api = spec::parse(include_str!("../../tests/fixtures/pets.yaml")).unwrap();
        let operation = &api.operations[0];
        let draft = Draft::new(operation)
            .edit(Field::Parameter(0), "a/b ?".into())
            .edit(Field::Query, "team: A&B".into())
            .cycle(operation, Field::Auth)
            .edit(Field::Secret, "my-token".into());
        let request = prepare(operation, &draft).unwrap();
        assert_eq!(
            request.url.as_str(),
            "http://localhost:8080/api/pets/a%2Fb%20%3F?view=full&team=A%26B"
        );
        assert_eq!(request.headers["authorization"], "Bearer my-token");
        assert!(request.headers["authorization"].is_sensitive());
        assert_eq!(request.headers["content-type"], "application/json");
        assert!(
            !destination(operation, &draft)
                .unwrap()
                .as_str()
                .contains("my-token")
        );
        assert!(documented(operation, 404));
        assert!(!documented(operation, 500));
    }

    #[test]
    fn serializes_query_array_separators_and_replaces_duplicate_api_keys() {
        let api = spec::parse(
            r#"
openapi: 3.1.0
servers: [{url: 'https://example.test'}]
security: [{token: []}]
components:
  securitySchemes:
    token: {type: apiKey, in: query, name: token}
paths:
  /pets:
    get:
      parameters:
        - {name: exploded, in: query, schema: {type: array}, example: [a, b]}
        - {name: joined, in: query, explode: false, schema: {type: array}, example: ['a,b', c]}
"#,
        )
        .unwrap();
        let operation = &api.operations[0];
        let draft = Draft::new(operation)
            .cycle(operation, Field::Auth)
            .edit(Field::Query, "token: stale".into())
            .edit(Field::Secret, "new key".into());
        let request = prepare(operation, &draft).unwrap();
        assert_eq!(
            request.url.query(),
            Some("exploded=a&exploded=b&joined=a%2Cb,c&token=new+key")
        );
        let draft = draft.edit(Field::Parameter(0), "not an array".into());
        assert!(prepare(operation, &draft).is_err());
    }

    #[test]
    fn encodes_basic_and_cookie_credentials_in_their_expected_headers() {
        let api = spec::parse(
            r#"
openapi: 3.1.0
servers: [{url: 'https://example.test'}]
security: [{session: []}]
components:
  securitySchemes:
    session: {type: apiKey, in: cookie, name: sid}
paths: {/pets: {get: {}}}
"#,
        )
        .unwrap();
        let operation = &api.operations[0];
        let draft = Draft::new(operation)
            .cycle(operation, Field::Auth)
            .edit(Field::Secret, "abc;=".into());
        assert_eq!(
            prepare(operation, &draft).unwrap().headers["cookie"],
            "sid=abc%3B%3D"
        );
        let draft = draft.cycle(operation, Field::Auth);
        assert!(prepare(operation, &draft).is_err());
        let draft = draft.edit(Field::Secret, "name:pass".into());
        assert_eq!(
            prepare(operation, &draft).unwrap().headers["authorization"],
            "Basic bmFtZTpwYXNz"
        );
    }

    #[test]
    fn rejects_invalid_destinations_headers_and_missing_paths() {
        let api = spec::parse(include_str!("../../tests/fixtures/pets.yaml")).unwrap();
        let operation = &api.operations[0];
        [
            "",
            "file:///tmp/data",
            "https://user:secret@localhost",
            "https://localhost/?token=abc",
        ]
        .into_iter()
        .for_each(|server| {
            assert!(
                prepare(
                    operation,
                    &Draft::new(operation).edit(Field::Server, server.into())
                )
                .is_err()
            )
        });
        [
            "Host: another.example",
            "Broken header",
            "Invalid Name: value",
        ]
        .into_iter()
        .for_each(|headers| {
            assert!(
                prepare(
                    operation,
                    &Draft::new(operation).edit(Field::Headers, headers.into())
                )
                .is_err()
            )
        });
        ["", ".."].into_iter().for_each(|path| {
            assert!(
                prepare(
                    operation,
                    &Draft::new(operation).edit(Field::Parameter(0), path.into())
                )
                .is_err()
            )
        });
    }
}
