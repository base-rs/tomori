// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which labels of an answer a client keeps.
use std::collections::BTreeSet;

/// The labels a client keeps of every answer, removing the rest before the answer leaves it.
///
/// Labels can carry URLs, user identifiers or secrets, so a server side that sends what it reads to clients, or describes it to the agent, keeps only those it needs. Without a filter, a client keeps every label. `__name__`, the metric's name, is a label like any other, kept only if listed. Series whose remaining labels are the same stay apart, as Prometheus answered them.
///
/// ```
/// use tomori_prometheus::LabelFilter;
///
/// let filter = LabelFilter::only(["namespace", "pod"]);
/// assert!(filter.keeps("pod"));
/// assert!(!filter.keeps("url"));
/// assert!(LabelFilter::all().keeps("url"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelFilter {
    /// The names kept, or `None` to keep every label.
    names: Option<BTreeSet<String>>,
}

impl LabelFilter {
    /// Keep every label, as a client does without a filter.
    pub fn all() -> Self {
        Self::default()
    }

    /// Keep only the labels named.
    pub fn only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: Some(names.into_iter().map(Into::into).collect()),
        }
    }

    /// Whether the label `name` is kept.
    pub fn keeps(&self, name: &str) -> bool {
        self.names.as_ref().is_none_or(|names| names.contains(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_keeps_only_what_it_lists_or_everything() {
        let filter = LabelFilter::only(["pod".to_owned()]);
        assert!(filter.keeps("pod"));
        assert!(!filter.keeps("__name__"));
        assert!(LabelFilter::all().keeps("__name__"));
        assert!(!LabelFilter::only(Vec::<String>::new()).keeps("pod"));
    }
}
