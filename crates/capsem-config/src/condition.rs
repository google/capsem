use crate::types::PolicySubject;

#[derive(Debug, Clone)]
pub struct CompiledCondition {
    clauses: Vec<ConditionClause>,
}

#[derive(Debug, Clone)]
struct ConditionClause {
    atoms: Vec<ConditionAtom>,
}

#[derive(Debug, Clone)]
enum ConditionAtom {
    Has {
        field: String,
    },
    StringMethod {
        field: String,
        method: StringMethod,
    },
    ContainsPii {
        field: String,
    },
    Comparison {
        field: String,
        operator: ComparisonOperator,
        expected: String,
    },
}

#[derive(Debug, Clone)]
enum StringMethod {
    Matches { regex: regex::Regex },
    Contains { expected: String },
    EndsWith { expected: String },
    StartsWith { expected: String },
}

impl CompiledCondition {
    pub(super) fn parse_with<F>(condition: &str, validate: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Result<(), String>,
    {
        let clauses = parse_clauses(condition, &validate)?;
        if clauses.is_empty() {
            return Err("policy condition must not be empty".into());
        }
        Ok(Self { clauses })
    }

    pub fn evaluate<S>(&self, subject: &S) -> Result<bool, String>
    where
        S: PolicySubject + ?Sized,
    {
        for clause in &self.clauses {
            let mut all_atoms_match = true;
            for atom in &clause.atoms {
                if !atom.evaluate(subject)? {
                    all_atoms_match = false;
                    break;
                }
            }
            if all_atoms_match {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn parse_clauses<F>(condition: &str, validate: &F) -> Result<Vec<ConditionClause>, String>
where
    F: Fn(&str) -> Result<(), String>,
{
    let condition = strip_outer_grouping(condition.trim())?;
    let raw_clauses = split_disjunction(condition)?;
    if raw_clauses.len() > 1 {
        let mut clauses = Vec::new();
        for clause in raw_clauses {
            clauses.extend(parse_clauses(clause, validate)?);
        }
        return Ok(clauses);
    }

    let raw_terms = split_conjunction(condition)?;
    if raw_terms.is_empty() {
        return Err("policy condition contains an empty CEL term".into());
    }
    let mut clauses = vec![ConditionClause { atoms: Vec::new() }];
    for term in raw_terms {
        let term = strip_outer_grouping(term.trim())?;
        if contains_top_level_operator(term, "||")? || contains_top_level_operator(term, "&&")? {
            let nested = parse_clauses(term, validate)?;
            let mut expanded = Vec::new();
            for existing in &clauses {
                for nested_clause in &nested {
                    let mut atoms = existing.atoms.clone();
                    atoms.extend(nested_clause.atoms.clone());
                    expanded.push(ConditionClause { atoms });
                }
            }
            clauses = expanded;
        } else {
            let atom = ConditionAtom::parse_with(term, validate)?;
            for clause in &mut clauses {
                clause.atoms.push(atom.clone());
            }
        }
    }
    Ok(clauses)
}

impl ConditionAtom {
    fn parse_with<F>(atom: &str, validate: &F) -> Result<Self, String>
    where
        F: Fn(&str) -> Result<(), String>,
    {
        if let Some(inner) = atom.strip_prefix("has(").and_then(|s| s.strip_suffix(')')) {
            let field = inner.trim();
            validate(field)?;
            validate_field_name(field)?;
            return Ok(Self::Has {
                field: field.to_string(),
            });
        }

        for method in ["matches", "contains", "endsWith", "startsWith"] {
            if let Some((field, argument)) = parse_method_call(atom, method)? {
                validate(field)?;
                validate_field_name(field)?;
                let expected = parse_string_literal(argument)?;
                let method = match method {
                    "matches" => StringMethod::Matches {
                        regex: regex::Regex::new(&expected).map_err(|e| format!("invalid CEL matches() regex: {e}"))?,
                    },
                    "contains" => StringMethod::Contains { expected },
                    "endsWith" => StringMethod::EndsWith { expected },
                    "startsWith" => StringMethod::StartsWith { expected },
                    _ => unreachable!("method list is exhaustive"),
                };
                return Ok(Self::StringMethod {
                    field: field.to_string(),
                    method,
                });
            }
        }

        if let Some(field) = parse_zero_arg_method_call(atom, "contains_pii")? {
            validate(field)?;
            validate_field_name(field)?;
            return Ok(Self::ContainsPii {
                field: field.to_string(),
            });
        }

        if let Some((field, operator, value)) = parse_comparison(atom)? {
            validate(field)?;
            validate_field_name(field)?;
            return Ok(Self::Comparison {
                field: field.to_string(),
                operator,
                expected: parse_string_literal(value)?,
            });
        }

        Err(format!("unsupported CEL condition term: {atom}"))
    }

    fn evaluate<S>(&self, subject: &S) -> Result<bool, String>
    where
        S: PolicySubject + ?Sized,
    {
        match self {
            Self::Has { field } => Ok(subject.get_policy_field(field).is_some()),
            Self::StringMethod { field, method } => {
                let Some(actual) = subject
                    .get_policy_field(field)
                    .and_then(|value| value.as_string().map(str::to_owned))
                else {
                    return Ok(false);
                };
                Ok(match method {
                    StringMethod::Matches { regex } => regex.is_match(&actual),
                    StringMethod::Contains { expected } => actual.contains(expected),
                    StringMethod::EndsWith { expected } => actual.ends_with(expected),
                    StringMethod::StartsWith { expected } => actual.starts_with(expected),
                })
            }
            Self::ContainsPii { field } => {
                let Some(actual) = subject
                    .get_policy_field(field)
                    .and_then(|value| value.as_string().map(str::to_owned))
                else {
                    return Ok(false);
                };
                Ok(looks_like_pii(&actual))
            }
            Self::Comparison {
                field,
                operator,
                expected,
            } => {
                let Some(actual) = subject
                    .get_policy_field(field)
                    .and_then(|value| value.as_string().map(str::to_owned))
                else {
                    return Ok(false);
                };
                let matches = actual == *expected;
                Ok(match operator {
                    ComparisonOperator::Eq => matches,
                    ComparisonOperator::NotEq => !matches,
                })
            }
        }
    }
}

pub(crate) fn validate_condition_with<F>(condition: &str, validate: F) -> Result<(), String>
where
    F: Fn(&str) -> Result<(), String>,
{
    CompiledCondition::parse_with(condition, validate).map(|_| ())
}

pub(crate) fn evaluate_condition_with<S, F>(condition: &str, subject: &S, validate: F) -> Result<bool, String>
where
    S: PolicySubject + ?Sized,
    F: Fn(&str) -> Result<(), String>,
{
    CompiledCondition::parse_with(condition, validate)?.evaluate(subject)
}

fn split_disjunction(condition: &str) -> Result<Vec<&str>, String> {
    split_top_level_operator(condition, "||")
}

fn split_conjunction(condition: &str) -> Result<Vec<&str>, String> {
    split_top_level_operator(condition, "&&")
}

fn contains_top_level_operator(condition: &str, operator: &str) -> Result<bool, String> {
    Ok(split_top_level_operator(condition, operator)?.len() > 1)
}

fn split_top_level_operator<'a>(condition: &'a str, operator: &str) -> Result<Vec<&'a str>, String> {
    let mut atoms = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let bytes = condition.as_bytes();
    let operator_bytes = operator.as_bytes();
    let mut i = 0;

    // Scanned byte-wise on purpose: every byte this loop reacts to is ASCII, and
    // a UTF-8 continuation byte never collides with ASCII, so a multi-byte
    // character is skipped harmlessly. Slicing `condition` at `i` instead would
    // panic the moment a rule condition carried a non-ASCII character.
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
            _ if paren_depth == 0 && bytes[i..].starts_with(operator_bytes) => {
                let atom = condition[start..i].trim();
                if atom.is_empty() {
                    return Err("policy condition contains an empty CEL term".into());
                }
                atoms.push(atom);
                i += operator.len();
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

fn strip_outer_grouping(mut value: &str) -> Result<&str, String> {
    loop {
        let trimmed = value.trim();
        if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
            return Ok(trimmed);
        }
        if outer_parens_wrap(trimmed)? {
            value = &trimmed[1..trimmed.len() - 1];
        } else {
            return Ok(trimmed);
        }
    }
}

fn outer_parens_wrap(value: &str) -> Result<bool, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let bytes = value.as_bytes();

    for (index, byte) in bytes.iter().enumerate() {
        let ch = *byte as char;
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth
                    .checked_sub(1)
                    .ok_or_else(|| "policy condition has unmatched ')'".to_string())?;
                if paren_depth == 0 && index != bytes.len() - 1 {
                    return Ok(false);
                }
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("policy condition has an unterminated string literal".into());
    }
    if paren_depth != 0 {
        return Err("policy condition has unmatched '('".into());
    }
    Ok(true)
}

fn parse_method_call<'a>(atom: &'a str, method: &str) -> Result<Option<(&'a str, &'a str)>, String> {
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

fn parse_zero_arg_method_call<'a>(atom: &'a str, method: &str) -> Result<Option<&'a str>, String> {
    let needle = format!(".{method}(");
    let Some(index) = atom.find(&needle) else {
        return Ok(None);
    };
    let field = atom[..index].trim();
    let rest = atom[index + needle.len()..].trim();
    if rest != ")" {
        return Err(format!("CEL {method}() does not accept arguments"));
    }
    if field.is_empty() {
        return Err(format!("CEL {method}() call is missing its receiver"));
    }
    Ok(Some(field))
}

/// Compiled once. `contains_pii()` runs per security event, so building this
/// regex inside the call put regex compilation on the enforcement hot path.
static SSN_PATTERN: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("PII regex is valid"));

fn looks_like_pii(value: &str) -> bool {
    value.contains('@') || SSN_PATTERN.is_match(value)
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

/// A field name must be present and shaped like a dotted path; `has()` or
/// `== "x"` with nothing in front compiled to an atom that could never match.
fn validate_field_name(field: &str) -> Result<(), String> {
    if field.is_empty() {
        return Err("policy condition names an empty field".into());
    }
    if field.split('.').any(|segment| segment.is_empty())
        || !field
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
    {
        return Err(format!("policy condition field {field:?} is not a dotted identifier"));
    }
    Ok(())
}

/// Decode one quoted CEL string literal.
///
/// Rule generators emit JSON-escaped literals and rule authors write CEL
/// escapes; both must compare against the decoded value, or a rule whose value
/// holds a quote, a backslash, or a newline compiles and never matches. Only
/// the escapes that JSON, CEL and the regex engine all read the same way are
/// decoded: `\\ \" \' \/ \n \t \r \xHH \uHHHH \UHHHHHHHH`. Every other
/// backslash sequence is kept verbatim, because the shipped rules write regex
/// classes as `\.` and `\d` inside `matches()` literals and a regex reads
/// `\b` as a word boundary, not a backspace. So `"\\."` and `"\."` both reach
/// the regex engine as `\.`.
fn parse_string_literal(value: &str) -> Result<String, String> {
    let mut chars = value.trim().chars();
    let quote = match chars.next() {
        Some(quote @ ('\'' | '"')) => quote,
        _ => return Err("CEL comparison value must be a string literal".into()),
    };

    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == quote {
            if !chars.as_str().trim().is_empty() {
                return Err("CEL string literal has trailing content".into());
            }
            return Ok(out);
        }
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(escape) = chars.next() else { break };
        let decoded = match escape {
            '\\' | '"' | '\'' | '/' => escape,
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'x' => char_from_code(take_hex(&mut chars, 2)?)?,
            'u' => {
                let code = take_hex(&mut chars, 4)?;
                if (0xD800..=0xDBFF).contains(&code) {
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err("CEL string literal has a lone high surrogate escape".into());
                    }
                    let low = take_hex(&mut chars, 4)?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err("CEL string literal has an invalid surrogate pair escape".into());
                    }
                    char_from_code(0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00))?
                } else {
                    char_from_code(code)?
                }
            }
            'U' => char_from_code(take_hex(&mut chars, 8)?)?,
            other => {
                out.push('\\');
                other
            }
        };
        out.push(decoded);
    }

    Err("policy condition has an unterminated string literal".into())
}

fn take_hex(chars: &mut std::str::Chars<'_>, count: usize) -> Result<u32, String> {
    let mut code = 0u32;
    for _ in 0..count {
        let digit = chars
            .next()
            .and_then(|ch| ch.to_digit(16))
            .ok_or_else(|| format!("CEL string literal escape needs {count} hex digits"))?;
        code = code * 16 + digit;
    }
    Ok(code)
}

fn char_from_code(code: u32) -> Result<char, String> {
    char::from_u32(code).ok_or_else(|| format!("CEL string literal escape U+{code:04X} is not a character"))
}

#[cfg(test)]
mod tests;
