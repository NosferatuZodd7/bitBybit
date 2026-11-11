//! Request parameters as a short, secret-free string for the audit log: properties whose name looks secret are
//! masked whatever their value, strings pass through the redactor, the result is truncated.

use bitbybit_security::redact::{MASK, redact};
use serde_json::Value;

const SECRET_FRAGMENTS: &[&str] =
    &["password", "passphrase", "passwd", "pwd", "psk", "secret", "token", "key", "credential"];

pub fn is_secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECRET_FRAGMENTS.iter().any(|f| lower.contains(f))
}

fn write(v: &Value, out: &mut String, depth: usize) {
    if depth > crate::json::MAX_DEPTH {
        out.push('…');
        return;
    }
    match v {
        Value::Object(o) => {
            out.push('{');
            for (i, (k, v)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(k);
                out.push(':');
                if is_secret_name(k) && !v.is_null() {
                    out.push_str(MASK);
                } else {
                    write(v, out, depth + 1);
                }
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if i >= 32 {
                    out.push('…');
                    break;
                }
                write(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::String(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        other => out.push_str(&other.to_string()),
    }
}

pub fn describe(v: &Value, max_chars: usize) -> String {
    let mut raw = String::new();
    write(v, &mut raw, 0);
    let text = redact(&raw);
    if max_chars > 0 && text.chars().count() > max_chars {
        format!("{}…", text.chars().take(max_chars).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_secret_named_properties_and_secret_text() {
        let v = json!({"ssid": "Casa", "passphrase": "hunter2hunter2", "nested": {"apiKey": "abc", "note": "password=letmein"}, "list": [1, 2]});
        let d = describe(&v, 0);
        assert!(!d.contains("hunter2") && !d.contains("abc\"") && !d.contains("letmein"), "{d}");
        assert!(d.contains("ssid:\"Casa\"") && d.contains("list:[1,2]"));
        assert!(describe(&json!({"a": "x".repeat(100)}), 20).ends_with('…'));
        assert!(is_secret_name("wifiPsk") && !is_secret_name("ssid"));
    }
}
