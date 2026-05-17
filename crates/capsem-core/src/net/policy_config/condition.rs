use super::types::{PolicyCallback, PolicySubject};

pub(super) fn validate_policy_condition(
    callback: PolicyCallback,
    condition: &str,
) -> Result<(), String> {
    let atoms = split_conjunction(condition)?;
    if atoms.is_empty() {
        return Err("policy condition must not be empty".into());
    }
    for atom in atoms {
        validate_atom(callback, atom)?;
    }
    Ok(())
}

pub(super) fn evaluate_policy_condition<S>(
    callback: PolicyCallback,
    condition: &str,
    subject: &S,
) -> Result<bool, String>
where
    S: PolicySubject + ?Sized,
{
    let atoms = split_conjunction(condition)?;
    if atoms.is_empty() {
        return Err("policy condition must not be empty".into());
    }
    for atom in atoms {
        if !evaluate_atom(callback, atom, subject)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn split_conjunction(condition: &str) -> Result<Vec<&str>, String> {
    let mut atoms = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let bytes = condition.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth
                    .checked_sub(1)
                    .ok_or_else(|| "policy condition has unmatched ')'".to_string())?;
            }
            '&' if paren_depth == 0 && bytes.get(i + 1) == Some(&b'&') => {
                let atom = condition[start..i].trim();
                if atom.is_empty() {
                    return Err("policy condition contains an empty CEL term".into());
                }
                atoms.push(atom);
                i += 2;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    if quote.is_some() {
        return Err("policy condition has an unterminated string literal".into());
    }
    if paren_depth != 0 {
        return Err("policy condition has unmatched '('".into());
    }

    let atom = condition[start..].trim();
    if atom.is_empty() {
        return Err("policy condition contains an empty CEL term".into());
    }
    atoms.push(atom);
    Ok(atoms)
}

fn validate_atom(callback: PolicyCallback, atom: &str) -> Result<(), String> {
    if let Some(inner) = atom.strip_prefix("has(").and_then(|s| s.strip_suffix(')')) {
        let field = inner.trim();
        validate_field(callback, field)?;
        return Ok(());
    }

    for method in ["matches", "contains", "endsWith", "startsWith"] {
        if let Some((field, argument)) = parse_method_call(atom, method)? {
            validate_field(callback, field)?;
            let value = parse_string_literal(argument)?;
            if method == "matches" {
                regex::Regex::new(&value)
                    .map_err(|e| format!("invalid CEL matches() regex: {e}"))?;
            }
            return Ok(());
        }
    }

    if let Some((field, _operator, value)) = parse_comparison(atom)? {
        validate_field(callback, field)?;
        parse_string_literal(value)?;
        return Ok(());
    }

    Err(format!("unsupported CEL condition term: {atom}"))
}

fn evaluate_atom<S>(callback: PolicyCallback, atom: &str, subject: &S) -> Result<bool, String>
where
    S: PolicySubject + ?Sized,
{
    if let Some(inner) = atom.strip_prefix("has(").and_then(|s| s.strip_suffix(')')) {
        let field = inner.trim();
        validate_field(callback, field)?;
        return Ok(subject.get_policy_field(field).is_some());
    }

    for method in ["matches", "contains", "endsWith", "startsWith"] {
        if let Some((field, argument)) = parse_method_call(atom, method)? {
            validate_field(callback, field)?;
            let expected = parse_string_literal(argument)?;
            let Some(actual) = subject
                .get_policy_field(field)
                .and_then(|value| value.as_string().map(str::to_owned))
            else {
                return Ok(false);
            };
            return match method {
                "matches" => {
                    let regex = regex::Regex::new(&expected)
                        .map_err(|e| format!("invalid CEL matches() regex: {e}"))?;
                    Ok(regex.is_match(&actual))
                }
                "contains" => Ok(actual.contains(&expected)),
                "endsWith" => Ok(actual.ends_with(&expected)),
                "startsWith" => Ok(actual.starts_with(&expected)),
                _ => unreachable!("method list is exhaustive"),
            };
        }
    }

    if let Some((field, operator, value)) = parse_comparison(atom)? {
        validate_field(callback, field)?;
        let expected = parse_string_literal(value)?;
        let Some(actual) = subject
            .get_policy_field(field)
            .and_then(|value| value.as_string().map(str::to_owned))
        else {
            return Ok(false);
        };
        let matches = actual == expected;
        return Ok(match operator {
            ComparisonOperator::Eq => matches,
            ComparisonOperator::NotEq => !matches,
        });
    }

    Err(format!("unsupported CEL condition term: {atom}"))
}

fn parse_method_call<'a>(
    atom: &'a str,
    method: &str,
) -> Result<Option<(&'a str, &'a str)>, String> {
    let needle = format!(".{method}(");
    let Some(index) = atom.find(&needle) else {
        return Ok(None);
    };
    let field = atom[..index].trim();
    let rest = atom[index + needle.len()..].trim();
    let Some(argument) = rest.strip_suffix(')') else {
        return Err(format!("CEL {method}() call is missing ')'"));
    };
    if field.is_empty() {
        return Err(format!("CEL {method}() call is missing its receiver"));
    }
    Ok(Some((field, argument.trim())))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonOperator {
    Eq,
    NotEq,
}

fn parse_comparison(atom: &str) -> Result<Option<(&str, ComparisonOperator, &str)>, String> {
    if let Some(index) = find_operator(atom, "==")? {
        return Ok(Some((
            atom[..index].trim(),
            ComparisonOperator::Eq,
            atom[index + 2..].trim(),
        )));
    }
    if let Some(index) = find_operator(atom, "!=")? {
        return Ok(Some((
            atom[..index].trim(),
            ComparisonOperator::NotEq,
            atom[index + 2..].trim(),
        )));
    }
    Ok(None)
}

fn find_operator(atom: &str, operator: &str) -> Result<Option<usize>, String> {
    let mut quote = None;
    let mut escaped = false;
    let bytes = atom.as_bytes();
    let operator = operator.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(operator) {
            return Ok(Some(i));
        }
        i += 1;
    }

    if quote.is_some() {
        return Err("policy condition has an unterminated string literal".into());
    }
    Ok(None)
}

fn parse_string_literal(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() < 2 {
        return Err("CEL comparison value must be a string literal".into());
    }

    let quote = value.as_bytes()[0] as char;
    if quote != '\'' && quote != '"' {
        return Err("CEL comparison value must be a string literal".into());
    }

    let mut escaped = false;
    for (index, ch) in value[1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            let close = index + 1;
            if !value[close + 1..].trim().is_empty() {
                return Err("CEL string literal has trailing content".into());
            }
            return Ok(value[1..close].to_string());
        }
    }

    Err("policy condition has an unterminated string literal".into())
}

fn validate_field(callback: PolicyCallback, field: &str) -> Result<(), String> {
    if !is_valid_field_path(field) {
        return Err(format!("invalid CEL field path: {field}"));
    }
    if field_allowed(callback, field) {
        return Ok(());
    }
    Err(format!(
        "field '{field}' is not available on policy callback {:?}",
        callback
    ))
}

fn is_valid_field_path(field: &str) -> bool {
    !field.is_empty()
        && field.split('.').all(|part| {
            let mut chars = part.chars();
            matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
                && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        })
}

fn field_allowed(callback: PolicyCallback, field: &str) -> bool {
    let (exact, prefixes): (&[&str], &[&str]) = match callback {
        PolicyCallback::McpRequest => (
            &[
                "method",
                "request.id",
                "server.name",
                "tool.name",
                "resource.uri",
            ],
            &["arguments"],
        ),
        PolicyCallback::McpResponse => (
            &[
                "method",
                "request.id",
                "server.name",
                "tool.name",
                "response.text",
                "response.content",
                "response.is_error",
            ],
            &["arguments", "response"],
        ),
        PolicyCallback::HttpRequest => (
            &[
                "request.scheme",
                "request.host",
                "request.method",
                "request.path",
                "request.query",
                "request.url",
            ],
            &["request.headers"],
        ),
        PolicyCallback::HttpResponse => (
            &[
                "request.scheme",
                "request.host",
                "request.method",
                "request.path",
                "request.query",
                "request.url",
                "response.status",
                "response.body",
                "response.text",
            ],
            &["request.headers", "response.headers"],
        ),
        PolicyCallback::DnsQuery => (
            &["qname", "qtype", "protocol", "process.name"],
            &[] as &[&str],
        ),
        PolicyCallback::DnsResponse => (
            &["qname", "qtype", "rcode", "protocol", "process.name"],
            &["answer"],
        ),
        PolicyCallback::ModelRequest => (
            &[
                "provider",
                "model",
                "system_prompt",
                "request.data",
                "request.body",
                "messages_count",
                "tools_count",
            ],
            &["request.headers", "messages"],
        ),
        PolicyCallback::ModelResponse => (
            &[
                "provider",
                "model",
                "response.text",
                "text",
                "content",
                "thinking_content",
                "stop_reason",
            ],
            &["response"],
        ),
        PolicyCallback::ModelToolCall => (
            &["provider", "model", "tool.name", "tool.call_id"],
            &["tool.arguments"],
        ),
        PolicyCallback::ModelToolResponse => (
            &[
                "provider",
                "model",
                "tool.name",
                "tool.call_id",
                "content",
                "response.content",
                "is_error",
            ],
            &["tool.arguments", "response"],
        ),
        PolicyCallback::HookDecision => (
            &["callback", "decision", "rule.id", "endpoint.id"],
            &["request", "response"],
        ),
    };

    exact.contains(&field)
        || prefixes
            .iter()
            .any(|prefix| field == *prefix || field.starts_with(&format!("{prefix}.")))
}
