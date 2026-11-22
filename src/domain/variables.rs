use regex::Regex;
use std::{borrow::Cow, collections::BTreeMap, sync::LazyLock};

pub const CONFIG_LIMIT: usize = 64 * 1024;
pub const VALUE_LIMIT: usize = 1024 * 1024;

static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{[ \t]*([A-Za-z0-9_.-]+)[ \t]*\}\}").unwrap());

#[derive(Default)]
pub struct Variables(BTreeMap<String, String>);

impl Variables {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > CONFIG_LIMIT {
            return Err("Variables configuration exceeds 64 KiB".into());
        }
        let values: BTreeMap<String, String> = serde_json::from_str(text).map_err(|error| {
            format!(
                "Use a JSON object with string values (line {}, column {})",
                error.line(),
                error.column()
            )
        })?;
        if values.keys().any(|key| {
            key.is_empty()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        }) {
            return Err("Variable names use letters, digits, underscores, dots, or hyphens".into());
        }
        let variables = Self(values);
        if variables.json().len() > CONFIG_LIMIT {
            return Err("Variables configuration exceeds 64 KiB when formatted".into());
        }
        Ok(variables)
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(&self.0).expect("string values can always be encoded as JSON")
    }

    pub fn expand<'a>(&self, source: &'a str) -> Result<Cow<'a, str>, String> {
        if !source.contains("{{") {
            return Ok(Cow::Borrowed(source));
        }
        let (mut text, end) = TOKEN.captures_iter(source).try_fold(
            (String::new(), 0),
            |(mut text, end), captures| {
                let token = captures.get(0).expect("the matched token is captured");
                let key = &captures[1];
                let value = self
                    .0
                    .get(key)
                    .ok_or_else(|| format!("Set variable {{{{{key}}}}} in Variables (v)"))?;
                let prefix = &source[end..token.start()];
                if text.len() + prefix.len() + value.len() > VALUE_LIMIT {
                    return Err("Expanded request value exceeds 1 MiB".to_owned());
                }
                text.push_str(prefix);
                text.push_str(value);
                Ok((text, token.end()))
            },
        )?;
        if end == 0 {
            return Ok(Cow::Borrowed(source));
        }
        if text.len() + source.len() - end > VALUE_LIMIT {
            return Err("Expanded request value exceeds 1 MiB".to_owned());
        }
        text.push_str(&source[end..]);
        Ok(Cow::Owned(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_repeated_tokens_once_and_reports_missing_names() {
        let variables = Variables::parse(r#"{"name":"Fern","literal":"{{name}}"}"#).unwrap();
        assert_eq!(
            variables.expand("{{name}}/{{ name }}/{{literal}}").unwrap(),
            "Fern/Fern/{{name}}"
        );
        assert_eq!(
            variables.expand("{{ literal braces }}").unwrap(),
            "{{ literal braces }}"
        );
        assert!(
            variables
                .expand("{{missing}}")
                .unwrap_err()
                .contains("{{missing}}")
        );
        assert!(Variables::parse(r#"{"bad key":"value"}"#).is_err());
        assert!(Variables::parse(r#"{"count":1}"#).is_err());
    }
}
