use serde_json::Value;
use std::{fmt, ops::ControlFlow};

const EXAMPLE_LIMIT: usize = 64 * 1024;
const FIELD_LIMIT: usize = 128;
const IMPORT_LIMIT: usize = 16 * 1024 * 1024;

const METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Path,
    Query,
    Header,
    Cookie,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Header => "header",
            Self::Cookie => "cookie",
        })
    }
}

#[derive(Clone, Copy)]
pub enum Encoding {
    Scalar,
    QueryArray { explode: bool },
    Manual,
}

pub struct Parameter {
    pub name: String,
    pub location: Location,
    pub required: bool,
    pub initial: String,
    pub encoding: Encoding,
}

pub struct Body {
    pub media_type: String,
    pub example: String,
    pub required: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    Basic,
    Bearer,
    ApiKey(Location),
}

pub struct AuthOption {
    pub label: String,
    pub kind: AuthKind,
    pub key: String,
}

pub struct Operation {
    pub method: String,
    pub path: String,
    pub summary: String,
    pub tag: String,
    pub servers: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub bodies: Vec<Body>,
    pub statuses: Vec<String>,
    pub auth: Vec<AuthOption>,
}

pub struct Api {
    pub title: String,
    pub operations: Vec<Operation>,
    pub warnings: Vec<String>,
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn resolve<'a>(root: &'a Value, value: &'a Value) -> Result<&'a Value, String> {
    let follow = |reference: &str| {
        let pointer = reference
            .strip_prefix('#')
            .ok_or("External reference requires a bundled spec")?;
        let decoded = percent_encoding::percent_decode_str(pointer)
            .decode_utf8()
            .map_err(|_| "Invalid reference encoding")?;
        root.pointer(&decoded).ok_or("Unresolved local reference")
    };
    match (0..32).try_fold(value, |value, _| match value.get("$ref") {
        None => ControlFlow::Break(Ok(value)),
        Some(reference) => match follow(text(reference)) {
            Ok(target) => ControlFlow::Continue(target),
            Err(error) => ControlFlow::Break(Err(error.to_owned())),
        },
    }) {
        ControlFlow::Break(result) => result,
        ControlFlow::Continue(_) => Err("Reference chain is cyclic or too long".into()),
    }
}

fn example<'a>(
    root: &'a Value,
    node: &'a Value,
    schema: &'a Value,
) -> Result<Option<&'a Value>, String> {
    if let Some(value) = node.get("example") {
        return Ok(Some(value));
    }
    if let Some(first) = node["examples"]
        .as_object()
        .and_then(|examples| examples.values().next())
    {
        return resolve(root, first).map(|example| example.get("value"));
    }
    let schema = resolve(root, schema)?;
    Ok(schema
        .get("example")
        .or_else(|| {
            schema["examples"]
                .as_array()
                .and_then(|examples| examples.first())
        })
        .or_else(|| schema.get("default")))
}

fn format_example(value: &Value, json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(value).unwrap_or_default();
    }
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn parameter(root: &Value, entry: &Value) -> Result<Parameter, String> {
    let entry = resolve(root, entry)?;
    let location = match text(&entry["in"]) {
        "path" => Location::Path,
        "query" => Location::Query,
        "header" => Location::Header,
        "cookie" => Location::Cookie,
        location => return Err(format!("Unsupported parameter location {location:?}")),
    };
    let name = text(&entry["name"]).to_owned();
    if name.is_empty() {
        return Err("Parameter is missing its name".into());
    }
    let schema = resolve(root, &entry["schema"])?;
    let kind = text(&schema["type"]);
    let composite = matches!(kind, "array" | "object")
        || schema.get("items").is_some()
        || schema.get("properties").is_some();
    let default_style = match location {
        Location::Path | Location::Header => "simple",
        Location::Query | Location::Cookie => "form",
    };
    let style = entry["style"].as_str().unwrap_or(default_style);
    let encoding = if entry.get("content").is_some() || style != default_style {
        Encoding::Manual
    } else if location == Location::Query && (kind == "array" || schema.get("items").is_some()) {
        Encoding::QueryArray {
            explode: entry["explode"].as_bool().unwrap_or(true),
        }
    } else if composite {
        Encoding::Manual
    } else {
        Encoding::Scalar
    };
    let initial = if matches!(encoding, Encoding::Manual) {
        String::new()
    } else {
        example(root, entry, schema)?.map_or_else(String::new, |value| match encoding {
            Encoding::QueryArray { .. } => value.to_string(),
            _ => format_example(value, false),
        })
    };
    if initial.len() > EXAMPLE_LIMIT {
        return Err("Parameter example exceeds the 64 KiB import limit".into());
    }
    Ok(Parameter {
        name,
        location,
        required: location == Location::Path || entry["required"] == true,
        initial,
        encoding,
    })
}

fn servers(values: &Value) -> Vec<String> {
    const URL_LIMIT: usize = 8192;
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|server| {
            let url = server["url"].as_str()?;
            let mut parts = url.split('{');
            let first = parts.next().unwrap_or_default();
            if first.len() > URL_LIMIT {
                return None;
            }
            parts.try_fold(first.to_owned(), |mut result, part| {
                let (name, tail) = part.split_once('}')?;
                let replacement = server["variables"][name]["default"].as_str()?;
                if result.len() + replacement.len() + tail.len() > URL_LIMIT {
                    return None;
                }
                result.push_str(replacement);
                result.push_str(tail);
                Some(result)
            })
        })
        .take(FIELD_LIMIT)
        .collect()
}

fn authentication(root: &Value, operation: &Value) -> Vec<AuthOption> {
    let declared = operation
        .get("security")
        .unwrap_or(&root["security"])
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|requirement| {
            // shortcut: one automatic scheme per alternative, use manual headers or query fields for combined requirements
            let (name, _) = requirement
                .as_object()
                .filter(|entry| entry.len() == 1)?
                .iter()
                .next()?;
            let scheme = resolve(root, &root["components"]["securitySchemes"][name]).ok()?;
            let kind = match (
                text(&scheme["type"]),
                text(&scheme["scheme"]).to_ascii_lowercase().as_str(),
                text(&scheme["in"]),
            ) {
                ("http", "basic", _) => AuthKind::Basic,
                ("http", "bearer", _) => AuthKind::Bearer,
                ("apiKey", _, "header") => AuthKind::ApiKey(Location::Header),
                ("apiKey", _, "query") => AuthKind::ApiKey(Location::Query),
                ("apiKey", _, "cookie") => AuthKind::ApiKey(Location::Cookie),
                _ => return None,
            };
            if matches!(kind, AuthKind::ApiKey(_)) && text(&scheme["name"]).is_empty() {
                return None;
            }
            Some(AuthOption {
                label: name.clone(),
                kind,
                key: text(&scheme["name"]).to_owned(),
            })
        })
        .take(FIELD_LIMIT)
        .collect::<Vec<_>>();
    let fallback = [("Basic", AuthKind::Basic), ("Bearer", AuthKind::Bearer)]
        .into_iter()
        .filter(|(_, kind)| !declared.iter().any(|option| option.kind == *kind))
        .map(|(label, kind)| AuthOption {
            label: label.to_owned(),
            kind,
            key: String::new(),
        })
        .collect::<Vec<_>>();
    declared.into_iter().chain(fallback).collect()
}

fn operation(root: &Value, item: &Value, method: &str, path: &str) -> Result<Operation, String> {
    let node = &item[method];
    if !node.is_object() {
        return Err("Operation must be an object".into());
    }
    let body = resolve(root, &node["requestBody"])?;
    let count = [item, node]
        .into_iter()
        .filter_map(|value| value["parameters"].as_array())
        .map(Vec::len)
        .sum::<usize>()
        + body["content"]
            .as_object()
            .map_or(0, |content| content.len());
    if count > FIELD_LIMIT {
        return Err("Operation exceeds the 128-field import limit".into());
    }
    let parameters = [item, node]
        .into_iter()
        .flat_map(|value| value["parameters"].as_array().into_iter().flatten())
        .map(|value| parameter(root, value))
        .try_fold(Vec::<Parameter>::new(), |mut parameters, next| {
            let next = next?;
            parameters.retain(|existing| {
                existing.location != next.location
                    || if existing.location == Location::Header {
                        !existing.name.eq_ignore_ascii_case(&next.name)
                    } else {
                        existing.name != next.name
                    }
            });
            parameters.push(next);
            Ok::<_, String>(parameters)
        })?;
    let bodies = body["content"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(media_type, content)| {
            let json = media_type == "application/json" || media_type.ends_with("+json");
            let example = example(root, content, &content["schema"])?
                .map_or_else(String::new, |value| format_example(value, json));
            if example.len() > EXAMPLE_LIMIT {
                return Err("Body example exceeds the 64 KiB import limit".into());
            }
            Ok(Body {
                media_type: media_type.clone(),
                example,
                required: body["required"] == true,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Operation {
        method: method.to_ascii_uppercase(),
        path: path.to_owned(),
        summary: text(&node["summary"]).to_owned(),
        tag: node["tags"]
            .as_array()
            .and_then(|tags| tags.first())
            .and_then(Value::as_str)
            .unwrap_or("Untagged")
            .to_owned(),
        servers: [node, item, root]
            .into_iter()
            .find_map(|entry| {
                let servers = servers(&entry["servers"]);
                (!servers.is_empty()).then_some(servers)
            })
            .unwrap_or_default(),
        parameters,
        bodies,
        statuses: node["responses"]
            .as_object()
            .into_iter()
            .flat_map(|entries| entries.keys().cloned())
            .collect(),
        auth: authentication(root, node),
    })
}

type Imported = (Vec<Operation>, Vec<String>, usize);

fn accumulate(
    (mut operations, mut warnings, size): Imported,
    result: Result<Operation, String>,
) -> Result<Imported, String> {
    let size = match result {
        Err(warning) => {
            let size = size + warning.len();
            warnings.push(warning);
            size
        }
        Ok(operation) => {
            let size = size
                + operation.path.len()
                + operation.summary.len()
                + operation.tag.len()
                + operation
                    .servers
                    .iter()
                    .chain(&operation.statuses)
                    .map(String::len)
                    .sum::<usize>()
                + operation
                    .parameters
                    .iter()
                    .map(|p| p.name.len() + p.initial.len())
                    .sum::<usize>()
                + operation
                    .bodies
                    .iter()
                    .map(|b| b.media_type.len() + b.example.len())
                    .sum::<usize>()
                + operation
                    .auth
                    .iter()
                    .map(|a| a.label.len() + a.key.len())
                    .sum::<usize>();
            operations.push(operation);
            size
        }
    };
    if size > IMPORT_LIMIT {
        return Err("Expanded OpenAPI operations exceed the 16 MiB import limit".into());
    }
    Ok((operations, warnings, size))
}

pub fn parse(source: &str) -> Result<Api, String> {
    let options = serde_saphyr::Options {
        strict_booleans: true,
        with_snippet: false,
        budget: Some(serde_saphyr::Budget {
            max_depth: 128,
            max_documents: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    let root: Value = serde_saphyr::from_str_with_options(source, options)
        .map_err(|error| format!("Cannot parse OpenAPI: {error}"))?;
    let version = text(&root["openapi"]);
    if !matches!(version.split('.').collect::<Vec<_>>().as_slice(), ["3", "0" | "1", patch] if patch.parse::<u32>().is_ok())
    {
        return Err(format!("Expected OpenAPI 3.0 or 3.1, found {version:?}"));
    }
    let paths = root["paths"]
        .as_object()
        .ok_or("The spec has no paths object")?;
    let (mut operations, warnings, _) = paths
        .iter()
        .filter(|(path, _)| path.starts_with('/'))
        .try_fold((Vec::new(), Vec::new(), 0usize), |state, (path, item)| {
            let item = match resolve(&root, item) {
                Ok(item) if item.is_object() => item,
                invalid => {
                    let error = invalid
                        .err()
                        .unwrap_or_else(|| "Path item must be an object".into());
                    return accumulate(state, Err(format!("{path}: {error}")));
                }
            };
            METHODS
                .iter()
                .filter(|&&method| item.get(method).is_some())
                .try_fold(state, |state, method| {
                    let operation = operation(&root, item, method, path).map_err(|error| {
                        format!("{} {path}: {error}", method.to_ascii_uppercase())
                    });
                    accumulate(state, operation)
                })
        })?;
    if operations.is_empty() {
        return Err(warnings
            .first()
            .cloned()
            .unwrap_or_else(|| "No HTTP operations found".into()));
    }
    operations.sort_by(|left, right| {
        (&left.tag, &left.path, &left.method).cmp(&(&right.tag, &right.path, &right.method))
    });
    Ok(Api {
        title: root["info"]["title"]
            .as_str()
            .unwrap_or("OpenAPI")
            .to_owned(),
        operations,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_yaml_and_json_with_inheritance_examples_and_references() {
        let yaml = include_str!("../../tests/fixtures/pets.yaml");
        let json = include_str!("../../tests/fixtures/pets.json");
        [yaml, json].into_iter().for_each(|source| {
            let api = parse(source).unwrap();
            let operation = &api.operations[0];
            assert_eq!(api.title, "Pet shelter");
            assert_eq!(operation.servers, ["http://localhost:8080/api"]);
            assert_eq!(operation.parameters.len(), 2);
            assert_eq!(operation.parameters[0].initial, "42");
            assert_eq!(operation.parameters[1].initial, "full");
            assert!(operation.bodies[0].example.contains("Fern"));
            assert!(operation.auth[0].kind == AuthKind::Bearer);
            assert_eq!(operation.statuses, ["200", "4XX"]);
        });
    }

    #[test]
    fn caps_expansion_from_reused_references() {
        let template = r##"{
            "openapi":"3.1.0",
            "components":{"pathItems":{"shared":{"get":{"requestBody":{
                "content":{"text/plain":{"example":"EXAMPLE"}}
            }}}}},
            "paths":{PATHS}
        }"##;
        let paths = (0..300)
            .map(|index| format!(r##""/{index}":{{"$ref":"#/components/pathItems/shared"}}"##))
            .collect::<Vec<_>>()
            .join(",");
        let source = template
            .replace("EXAMPLE", &"x".repeat(EXAMPLE_LIMIT))
            .replace("PATHS", &paths);
        let result = parse(&source);
        assert!(matches!(result, Err(error) if error.contains("16 MiB")));
    }

    #[test]
    fn reports_external_cycles_and_invalid_specs() {
        let api = parse("openapi: 3.1.0\npaths: {/pets: {get: {}}, /other: {$ref: other.yaml}, /cycle: {$ref: '#/paths/~1cycle'}}").unwrap();
        assert!(
            api.warnings
                .iter()
                .any(|warning| warning.contains("bundled"))
        );
        assert!(
            api.warnings
                .iter()
                .any(|warning| warning.contains("cyclic"))
        );
        assert!(parse("swagger: '2.0'").is_err());
        assert!(parse("openapi: [").is_err());
    }
}
