use super::spec::{AuthKind, AuthOption, Encoding, Location, Operation, Parameter};
use super::variables::{VALUE_LIMIT, Variables};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http::{HeaderMap, HeaderName, HeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::borrow::Cow;
use url::Url;

const URL_LIMIT: usize = 64 * 1024;
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn fits_url(bytes: usize) -> Result<(), String> {
    if bytes > URL_LIMIT {
        return Err("Request URL exceeds 64 KiB".into());
    }
    Ok(())
}

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
    // Fields the user saved from an editor; only their text gets saved variables filled in.
    edited: Vec<Field>,
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
            edited: Vec::new(),
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

    pub fn body_starter<'a>(&self, operation: &'a Operation) -> Option<&'a str> {
        if self.edited.contains(&Field::Body) || !self.body.is_empty() {
            return None;
        }
        operation
            .bodies
            .get(self.media)
            .map(|body| body.starter.as_str())
            .filter(|starter| !starter.is_empty())
    }

    fn unedited(self, field: Field) -> Self {
        Self {
            edited: self
                .edited
                .into_iter()
                .filter(|&edited| edited != field)
                .collect(),
            ..self
        }
    }

    // Spec-supplied text stays literal, so an untrusted spec cannot pull saved variables into a request.
    fn expand<'a>(
        &self,
        field: Field,
        value: &'a str,
        variables: &Variables,
    ) -> Result<Cow<'a, str>, String> {
        if !self.edited.contains(&field) {
            return Ok(value.into());
        }
        variables.expand(value)
    }

    pub fn edit(self, field: Field, value: String) -> Self {
        let draft = self.unedited(field);
        let draft = Self {
            edited: draft.edited.iter().copied().chain([field]).collect(),
            ..draft
        };
        draft.replace(field, value)
    }

    fn replace(self, field: Field, value: String) -> Self {
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
                self.unedited(field)
                    .replace(field, operation.servers[next].clone())
            }
            Field::Media => {
                let media = (self.media + 1) % (operation.bodies.len() + 1);
                let example = |index| {
                    operation
                        .bodies
                        .get(index)
                        .map_or("", |body: &super::spec::Body| body.example.as_str())
                };
                if self.body != example(self.media) {
                    return Self { media, ..self };
                }
                Self {
                    media,
                    body: example(media).to_owned(),
                    ..self.unedited(Field::Body)
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
        self.parameter_fields(operation)
            .map(|(_, parameter, value)| (parameter, value))
    }

    fn parameter_fields<'a>(
        &'a self,
        operation: &'a Operation,
    ) -> impl Iterator<Item = (Field, &'a Parameter, &'a str)> + Clone + 'a {
        operation
            .parameters
            .iter()
            .zip(self.parameters.iter().map(String::as_str))
            .enumerate()
            .filter(|(_, (_, value))| !value.is_empty())
            .map(|(index, (parameter, value))| (Field::Parameter(index), parameter, value))
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
    let server = if draft.edited.contains(&Field::Server) {
        variables.expand_server(&draft.server)?
    } else {
        draft.server.as_str().into()
    };
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
            fits_url(path.len() + segment.len() + 1)?;
            path.push('/');
            path.push_str(&segment);
            Ok(path)
        },
    )?;
    let mut url = server;
    url.set_path(&format!("{}{}", url.path().trim_end_matches('/'), path));
    fits_url(url.as_str().len())?;
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
            let (field, parameter, value) = draft
                .parameter_fields(operation)
                .find(|(_, param, _)| param.location == Location::Path && param.name == key)
                .ok_or_else(|| format!("Fill in path parameter {key}"))?;
            let expanded_value = draft.expand(field, value, variables)?;
            if expanded_value.is_empty() {
                return Err(format!("Fill in path parameter {key}"));
            }
            let value = match parameter.encoding {
                Encoding::Manual if !expanded_value.contains('/') => {
                    Cow::Borrowed(expanded_value.as_ref())
                }
                Encoding::Manual => {
                    return Err(format!("Encode '/' as %2F in path parameter {key}"));
                }
                _ => utf8_percent_encode(&expanded_value, COMPONENT).into(),
            };
            fits_url(expanded.len() + literal.len() + value.len())?;
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
    fits_url(query.len() + key.len() + escaped.len() + 2)?;
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

type Credential<'a> = (Location, &'a str, &'a str);
type Values<'a> = [(&'a Parameter, Cow<'a, str>)];

fn secret<'a>(
    auth: Option<&AuthOption>,
    draft: &'a Draft,
    variables: &Variables,
) -> Result<Cow<'a, str>, String> {
    if auth.is_none() {
        return Ok("".into());
    }
    let secret = variables.expand(&draft.secret)?;
    if secret.is_empty() {
        return Err("Enter credentials for the selected authentication scheme".into());
    }
    Ok(secret)
}

fn parameter_values<'a>(
    operation: &'a Operation,
    draft: &'a Draft,
    variables: &Variables,
    auth_key: Option<Credential<'_>>,
) -> Result<Vec<(&'a Parameter, Cow<'a, str>)>, String> {
    draft
        .parameter_fields(operation)
        .filter(|(_, parameter, _)| {
            parameter.location != Location::Path
                && !auth_key.is_some_and(|(location, key, _)| {
                    parameter.location == location && parameter.name == key
                })
        })
        .try_fold(
            (Vec::new(), 0),
            |(mut fields, bytes), (field, parameter, value)| -> Result<_, String> {
                let value = draft.expand(field, value, variables)?;
                let bytes = add_expanded_bytes(bytes, &value)?;
                if !value.is_empty() {
                    fields.push((parameter, value));
                }
                Ok((fields, bytes))
            },
        )
        .map(|(fields, _)| fields)
}

fn query_string(
    draft: &Draft,
    variables: &Variables,
    parameters: &Values<'_>,
    auth_key: Option<Credential<'_>>,
) -> Result<String, String> {
    let query = parameters
        .iter()
        .filter(|(parameter, _)| parameter.location == Location::Query)
        .try_fold(String::new(), |query, (parameter, value)| {
            query_field(query, parameter, value)
        })?;
    let query_auth = auth_key.filter(|(location, _, _)| *location == Location::Query);
    pairs(&draft.query, "Extra query", variables)?
        .into_iter()
        .filter(|(key, _)| !query_auth.is_some_and(|(_, name, _)| key == name))
        .chain(query_auth.map(|(_, key, value)| (key.to_owned(), value.to_owned())))
        .try_fold(query, |query, (key, value)| {
            append_query(query, &key, &encode_query(&value))
        })
}

fn request_headers<'a>(
    draft: &Draft,
    variables: &Variables,
    parameters: &'a Values<'a>,
    auth_key: Option<Credential<'a>>,
) -> Result<HeaderMap, String> {
    let typed = pairs(&draft.headers, "Headers", variables)?;
    let fields = parameters
        .iter()
        .map(|(parameter, value)| (parameter.location, parameter.name.as_str(), value.as_ref()));
    let mut headers = fields
        .clone()
        .filter(|(location, _, _)| *location == Location::Header)
        .map(|(_, key, value)| (key.to_owned(), value.to_owned()))
        .chain(typed)
        .map(header)
        .collect::<Result<HeaderMap, _>>()?;
    // Servers may read only the first Cookie header, so typed and documented cookies share one.
    let cookies = headers
        .get_all(http::header::COOKIE)
        .iter()
        .map(|value| value.as_bytes().to_vec())
        .chain(
            fields
                .chain(auth_key)
                .filter(|(location, _, _)| *location == Location::Cookie)
                .map(|(_, key, value)| cookie(key, value).into_bytes()),
        )
        .collect::<Vec<_>>()
        .join(&b"; "[..]);
    if cookies.is_empty() {
        return Ok(headers);
    }
    headers.insert(
        http::header::COOKIE,
        HeaderValue::from_bytes(&cookies).map_err(|_| "Invalid cookie")?,
    );
    Ok(headers)
}

fn authorization(
    auth: Option<&AuthOption>,
    secret: &str,
    auth_key: Option<Credential<'_>>,
) -> Result<Option<(String, String)>, String> {
    let Some(auth) = auth else {
        return Ok(None);
    };
    let value = match auth.kind {
        AuthKind::Basic if !secret.contains(':') => {
            return Err("Basic authentication uses username:password".into());
        }
        AuthKind::Basic => format!("Basic {}", STANDARD.encode(secret.as_bytes())),
        AuthKind::Bearer => format!("Bearer {secret}"),
        AuthKind::ApiKey(_) => {
            return Ok(auth_key
                .filter(|(location, _, _)| *location == Location::Header)
                .map(|(_, key, value)| (key.to_owned(), value.to_owned())));
        }
    };
    Ok(Some(("Authorization".to_owned(), value)))
}

pub fn prepare(
    operation: &Operation,
    draft: &Draft,
    variables: &Variables,
) -> Result<Request, String> {
    let mut url = destination(operation, draft, variables)?;
    let auth = draft.auth_option(operation);
    let secret = secret(auth, draft, variables)?;
    let auth_key = auth.and_then(|option| match option.kind {
        AuthKind::ApiKey(location) => Some((location, option.key.as_str(), secret.as_ref())),
        _ => None,
    });
    let parameters = parameter_values(operation, draft, variables, auth_key)?;
    let query = query_string(draft, variables, &parameters, auth_key)?;
    fits_url(url.as_str().len() + query.len() + usize::from(!query.is_empty()))?;
    if !query.is_empty() {
        url.set_query(Some(&query));
    }
    let mut headers = request_headers(draft, variables, &parameters, auth_key)?;
    if let Some(pair) = authorization(auth, &secret, auth_key)? {
        let (name, mut value) = header(pair)?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    let payload = draft.expand(Field::Body, &draft.body, variables)?;
    if let Some(body) = operation.bodies.get(draft.media)
        && !headers.contains_key(http::header::CONTENT_TYPE)
        // A media range such as */* describes what the server accepts; it is not a sendable type.
        && !body.media_type.contains('*')
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
    fn accepts_single_brace_server_variable_in_preview_and_request() {
        let api = spec::parse(include_str!("../../tests/fixtures/pets.yaml")).unwrap();
        let operation = &api.operations[0];
        let variables =
            Variables::parse(r#"{"base_url":"http://localhost:8080","bad":"file:///tmp"}"#)
                .unwrap();
        let draft = Draft::new(operation).edit(Field::Server, "{base_url}".into());
        let target = super::destination(operation, &draft, &variables).unwrap();
        assert_eq!(target.as_str(), "http://localhost:8080/pets/42");
        let request = super::prepare(operation, &draft, &variables).unwrap();
        assert_eq!(
            request.url.as_str(),
            "http://localhost:8080/pets/42?view=full"
        );
        assert_eq!(variables.expand("{base_url}").unwrap(), "{base_url}");

        let missing = draft.clone().edit(Field::Server, "{missing}".into());
        assert!(
            super::destination(operation, &missing, &variables)
                .unwrap_err()
                .contains("{missing}")
        );
        let invalid = draft.edit(Field::Server, "{bad}".into());
        assert!(super::destination(operation, &invalid, &variables).is_err());
    }

    #[test]
    fn resolves_draft_tokens_before_wire_encoding_and_keeps_header_boundaries() {
        let api = spec::parse(include_str!("../../tests/fixtures/pets.yaml")).unwrap();
        let operation = &api.operations[0];
        let variables = Variables::parse(
            r#"{
            "base_url":"https://example.test/v1",
            "pet":"fern/42",
            "account":"A&B",
            "token":"local-value",
            "bad":"x\r\nX-Injected: value"
        }"#,
        )
        .unwrap();
        let draft = Draft::new(operation)
            .edit(Field::Server, "{{base_url}}".into())
            .edit(Field::Parameter(0), "{{pet}}".into())
            .edit(Field::Headers, "X-Account: {{account}}".into())
            .edit(Field::Query, "account: {{account}}".into())
            .edit(Field::Body, r#"{"name":"{{pet}}"}"#.into())
            .cycle(operation, Field::Auth)
            .edit(Field::Secret, "{{token}}".into());

        let target = super::destination(operation, &draft, &variables).unwrap();
        let request = super::prepare(operation, &draft, &variables).unwrap();
        assert_eq!(target.as_str(), "https://example.test/v1/pets/fern%2F42");
        assert_eq!(request.url.query(), Some("view=full&account=A%26B"));
        assert_eq!(request.headers["x-account"], "A&B");
        assert_eq!(request.headers["authorization"], "Bearer local-value");
        assert_eq!(request.body, r#"{"name":"fern/42"}"#);
        assert_eq!(draft.server, "{{base_url}}");

        let invalid = draft
            .clone()
            .edit(Field::Headers, "X-Account: {{bad}}".into());
        assert!(
            super::prepare(operation, &invalid, &variables)
                .err()
                .unwrap()
                .contains("Invalid value")
        );
        let missing = draft.edit(Field::Server, "{{missing}}".into());
        assert!(
            super::prepare(operation, &missing, &variables)
                .err()
                .unwrap()
                .contains("{{missing}}")
        );
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
        let typed = draft.clone().edit(Field::Headers, "Cookie: other=1".into());
        let request = prepare(operation, &typed).unwrap();
        assert_eq!(
            request.headers.get_all("cookie").iter().collect::<Vec<_>>(),
            ["other=1; sid=abc%3B%3D"]
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
    fn keeps_spec_supplied_tokens_literal_until_the_user_edits_the_field() {
        let api = spec::parse(
            r#"
openapi: 3.1.0
servers: [{url: 'https://example.test'}]
paths:
  /pets:
    post:
      parameters: [{name: q, in: query, example: '{{token}}'}]
      requestBody: {content: {'*/*': {example: 'body={{token}}'}}}
"#,
        )
        .unwrap();
        let operation = &api.operations[0];
        let variables = Variables::parse(r#"{"token":"S3CRET"}"#).unwrap();
        let draft = Draft::new(operation);
        let request = super::prepare(operation, &draft, &variables).unwrap();
        assert!(!format!("{}{}", request.url, request.body).contains("S3CRET"));
        assert_eq!(request.body, "body={{token}}");
        // A media range is not a type the client can claim to send.
        assert!(!request.headers.contains_key("content-type"));

        let draft = draft
            .edit(Field::Parameter(0), "{{token}}".into())
            .edit(Field::Body, "{{token}}".into());
        let request = super::prepare(operation, &draft, &variables).unwrap();
        assert_eq!(request.url.query(), Some("q=S3CRET"));
        assert_eq!(request.body, "S3CRET");
        // Cycling back to a spec server makes the field spec-supplied again.
        let draft = draft
            .edit(Field::Server, "https://{{token}}.test".into())
            .cycle(operation, Field::Server);
        let request = super::prepare(operation, &draft, &variables).unwrap();
        assert_eq!(request.url.host_str(), Some("example.test"));
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
