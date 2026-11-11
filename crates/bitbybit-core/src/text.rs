//! User-facing text is a catalog key plus an English fallback and positional arguments (`{0}`, `{1}`…). Code
//! never contains UI language; `bitbybit-localization` resolves keys. The fallback is what logs and reports
//! without a catalog show.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedText {
    pub key: String,
    pub fallback: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl LocalizedText {
    pub fn new(key: impl Into<String>, fallback: impl Into<String>) -> Self {
        Self { key: key.into(), fallback: fallback.into(), args: Vec::new() }
    }

    pub fn with_args<I, S>(key: impl Into<String>, fallback: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: ToString,
    {
        Self { key: key.into(), fallback: fallback.into(), args: args.into_iter().map(|a| a.to_string()).collect() }
    }

    /// Renders with the English fallback.
    pub fn to_fallback_string(&self) -> String {
        format_template(&self.fallback, &self.args)
    }
}

impl fmt::Display for LocalizedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_fallback_string())
    }
}

/// `text!("key", "English {0}", arg…)` builds a [`LocalizedText`].
#[macro_export]
macro_rules! text {
    ($key:expr, $fallback:expr) => {
        $crate::text::LocalizedText::new($key, $fallback)
    };
    ($key:expr, $fallback:expr, $($arg:expr),+ $(,)?) => {
        $crate::text::LocalizedText {
            key: ::std::string::String::from($key),
            fallback: ::std::string::String::from($fallback),
            args: ::std::vec![$(::std::string::ToString::to_string(&$arg)),+],
        }
    };
}

/// Replaces `{n}` with `args[n]`; `{{` and `}}` are literal braces. A malformed template or a missing argument
/// never fails: the template is returned followed by the arguments, so a bad catalog entry cannot break the UI.
pub fn format_template(template: &str, args: &[String]) -> String {
    if args.is_empty() && !template.contains(['{', '}']) {
        return template.to_owned();
    }
    try_format(template, args).unwrap_or_else(|| {
        if args.is_empty() { template.to_owned() } else { format!("{template} [{}]", args.join(", ")) }
    })
}

fn try_format(template: &str, args: &[String]) -> Option<String> {
    let mut out = String::with_capacity(template.len() + args.iter().map(String::len).sum::<usize>());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut index = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        d if d.is_ascii_digit() => index.push(d),
                        _ => return None,
                    }
                }
                out.push_str(args.get(index.parse::<usize>().ok()?)?);
            }
            '}' => return None,
            other => out.push(other),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_positional_arguments() {
        let t = text!("error.notFound", "Not found: {0} on {1}", "Ethernet 2", 7);
        assert_eq!(t.to_fallback_string(), "Not found: Ethernet 2 on 7");
        assert_eq!(text!("k", "plain").to_fallback_string(), "plain");
        assert_eq!(format_template("{1}{0}{1}", &["a".into(), "b".into()]), "bab");
    }

    #[test]
    fn escaped_braces_are_literal() {
        assert_eq!(format_template("{{x}} {0}", &["v".into()]), "{x} v");
    }

    #[test]
    fn malformed_templates_do_not_fail() {
        assert_eq!(format_template("Broken {1", &["a".into()]), "Broken {1 [a]");
        assert_eq!(format_template("Missing {3}", &["a".into()]), "Missing {3} [a]");
        assert_eq!(format_template("No args {0}", &[]), "No args {0}");
    }

    #[test]
    fn json_shape_is_camel_case_without_empty_args() {
        let json = serde_json::to_string(&text!("a.b", "x")).unwrap();
        assert_eq!(json, r#"{"key":"a.b","fallback":"x"}"#);
        let back: LocalizedText = serde_json::from_str(r#"{"key":"a.b","fallback":"x {0}","args":["1"]}"#).unwrap();
        assert_eq!(back.to_string(), "x 1");
    }
}
