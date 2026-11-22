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
        values.keys().try_for_each(|key| Self::validate_key(key))?;
        Self(values).check_size()
    }

    fn validate_key(key: &str) -> Result<(), String> {
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        {
            return Err("Variable names use letters, digits, underscores, dots, or hyphens".into());
        }
        Ok(())
    }

    fn check_size(self) -> Result<Self, String> {
        if self.json().len() > CONFIG_LIMIT {
            return Err("Variables configuration exceeds 64 KiB when formatted".into());
        }
        Ok(self)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub fn check_name(&self, key: &str, previous: Option<&str>) -> Result<(), String> {
        Self::validate_key(key)?;
        if previous != Some(key) && self.0.contains_key(key) {
            return Err("That variable name already exists".into());
        }
        Ok(())
    }

    pub fn set(&self, previous: Option<&str>, key: String, value: String) -> Result<Self, String> {
        self.check_name(&key, previous)?;
        Self(self.without(previous).chain([(key, value)]).collect()).check_size()
    }

    pub fn remove(&self, key: &str) -> Self {
        Self(self.without(Some(key)).collect())
    }

    fn without<'a>(
        &'a self,
        previous: Option<&'a str>,
    ) -> impl Iterator<Item = (String, String)> + 'a {
        self.0
            .iter()
            .filter(move |(key, _)| previous != Some(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(&self.0).expect("string values can always be encoded as JSON")
    }

    pub fn expand_server<'a>(&'a self, source: &'a str) -> Result<Cow<'a, str>, String> {
        let token = source.trim();
        if let Some(key) = token
            .strip_prefix('{')
            .and_then(|key| key.strip_suffix('}'))
            && Self::validate_key(key.trim()).is_ok()
        {
            return self.token_value(key.trim(), token).map(Cow::Borrowed);
        }
        self.expand(source)
    }

    fn token_value(&self, key: &str, token: &str) -> Result<&str, String> {
        self.value(key)
            .ok_or_else(|| format!("Set variable {token} in Variables (v)"))
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
                let value = self.token_value(key, token.as_str())?;
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
    fn adding_or_renaming_to_an_existing_key_keeps_both_values() {
        let variables = Variables::parse(r#"{"first":"one","second":"two"}"#).unwrap();
        assert!(variables.set(None, "first".into(), "new".into()).is_err());
        assert!(
            variables
                .set(Some("second"), "first".into(), "new".into())
                .is_err()
        );
        assert_eq!(
            variables.entries().collect::<Vec<_>>(),
            [("first", "one"), ("second", "two")]
        );
    }

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
