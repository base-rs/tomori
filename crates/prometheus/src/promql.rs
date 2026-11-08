// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Building PromQL safely: quoting values read from elsewhere, and batching expressions within a gateway's bound on a query's size.

/// `value` as a PromQL string literal, in double quotes, so that it can only ever be a value in a selector, whatever it holds.
///
/// ```
/// use tomori_prometheus::promql::quote;
///
/// let pod = r#"web-1"} or vector(1) or {x=""#;
/// assert_eq!(format!("up{{pod={}}}", quote(pod)), r#"up{pod="web-1\"} or vector(1) or {x=\""}"#);
/// ```
pub fn quote(value: &str) -> String {
    // PromQL's strings take Go's escapes, of which JSON's are a subset.
    serde_json::to_string(value).expect("a string is always JSON")
}

/// `value` with every regular expression metacharacter escaped, so that it matches only itself in a `=~` or `!~` matcher, inside [`quote`]:
///
/// ```
/// use tomori_prometheus::promql::{escape_regex, quote};
///
/// let pods = ["web-1", "web.2"].map(escape_regex).join("|");
/// assert_eq!(format!("up{{pod=~{}}}", quote(&pods)), r#"up{pod=~"web-1|web\\.2"}"#);
/// ```
pub fn escape_regex(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if "\\.+*?()[]{}|^$".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// What joins two expressions: on a new line, which ends any comment before it.
const SEPARATOR: &str = "\nor ";

/// `expressions` joined with `or` into as few queries as fit in `max_bytes` each, in order, so that a poll needs fewer requests than expressions. An expression longer than `max_bytes` is a query of its own, which a client then refuses as too large.
///
/// Each `or` begins a line, so that a `# comment` ending an expression cannot comment out the expressions after it. Join only instant vectors: `or` refuses a scalar, such as `1` or `scalar(up)`, so wrap one in `vector()`; and it keeps only the left of two series with the same labels, so give each expression's series a label of its own, such as with `label_replace`. Samples of native histograms in a batch are left out of the answer with a warning, as everywhere (see [`Response`](crate::Response)), keeping the batch's other samples.
///
/// ```
/// use tomori_prometheus::promql::batch;
///
/// let queries = batch(&["a".repeat(6), "b # note".into(), "c".repeat(6)], 18);
/// assert_eq!(queries, ["aaaaaa\nor b # note", "cccccc"]);
/// ```
pub fn batch(expressions: &[String], max_bytes: usize) -> Vec<String> {
    let mut queries = Vec::new();
    let mut current = String::new();
    for expression in expressions {
        if !current.is_empty() && current.len() + SEPARATOR.len() + expression.len() > max_bytes {
            queries.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str(SEPARATOR);
        }
        current.push_str(expression);
    }
    if !current.is_empty() {
        queries.push(current);
    }
    queries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_quoted_and_escaped() {
        assert_eq!(quote("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
        assert_eq!(escape_regex("a.b|c(d)"), r"a\.b\|c\(d\)");
        assert_eq!(escape_regex("plain"), "plain");
    }

    #[test]
    fn expressions_are_batched_within_the_bound() {
        assert!(batch(&[], 10).is_empty());
        assert_eq!(batch(&["x".repeat(20)], 10), ["x".repeat(20)]);
        assert_eq!(
            batch(&["a".into(), "x".repeat(20), "b".into(), "c".into()], 10),
            ["a".to_owned(), "x".repeat(20), "b\nor c".into()]
        );
    }
}
