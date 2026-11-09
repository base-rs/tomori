// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a server side tells the space's agent of a metric it presents, when the agent inspects it.
use serde_json::{Value, json};

use crate::{Error, Labels, Point, Range, Timestamp};

/// When a query was evaluated: at an instant, or over a range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Window {
    Instant(Timestamp),
    Range(Range),
}

/// What a server side may tell the space's agent of a metric it presents, such as a panel's query, when the agent
/// inspects it through Tomori's `inspect` tool, so that the agent can say exactly what it shows, or ask the same of
/// Prometheus itself: the expression, when it was evaluated, the series' labels, the latest value, its unit and a link.
///
/// It has no field for the endpoint or the token, which stay in the [`Client`](crate::Client): the source is named
/// only by `datasource`, a name the application chooses, such as `production`. Its labels are those the side passes
/// on, which should be those its client's [`LabelFilter`](crate::LabelFilter) keeps, since labels can carry URLs, user
/// identifiers or secrets; and a link must not carry a credential either, which is why one with user information is
/// refused. See Tomori's `design/data-sources.md`.
///
/// ```
/// use std::time::Duration;
/// use tomori_prometheus::{Metric, Point, Range, Timestamp};
///
/// let end = Timestamp::from_millis(1_762_682_400_000);
/// let range = Range::new(end - Duration::from_secs(3600), end, Duration::from_secs(60));
/// let metric = Metric::range("production", "sum(rate(http_requests_total{code=~\"5..\"}[5m]))", range)
///     .with_latest(Point { at: end, value: 0.042 })
///     .with_unit("requests per second")
///     .with_link("https://grafana.example/d/checkout")
///     .unwrap();
/// let json = metric.to_json();
/// assert_eq!(json["kind"], "metric");
/// assert_eq!(json["datasource"], "production");
/// assert_eq!(json["range"]["start"], "2025-11-09T09:00:00.000Z");
/// assert_eq!(json["range"]["step_seconds"], 60.0);
/// assert_eq!(json["latest"]["value"], 0.042);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    datasource: String,
    query: String,
    window: Window,
    labels: Labels,
    latest: Option<Point>,
    unit: Option<String>,
    link: Option<String>,
}

/// `value` as JSON: a number, or Prometheus's own words for one JSON has none for.
fn number(value: f64) -> Value {
    if value.is_nan() {
        json!("NaN")
    } else if value.is_infinite() {
        json!(if value > 0.0 { "+Inf" } else { "-Inf" })
    } else {
        json!(value)
    }
}

impl Metric {
    /// The metric `query` evaluated at `at`, read from the source the application calls `datasource`.
    pub fn instant(datasource: &str, query: &str, at: Timestamp) -> Self {
        Self::new(datasource, query, Window::Instant(at))
    }

    /// The metric `query` evaluated over `range`, read from the source the application calls `datasource`.
    pub fn range(datasource: &str, query: &str, range: Range) -> Self {
        Self::new(datasource, query, Window::Range(range))
    }

    fn new(datasource: &str, query: &str, window: Window) -> Self {
        Self {
            datasource: datasource.to_owned(),
            query: query.to_owned(),
            window,
            labels: Labels::new(),
            latest: None,
            unit: None,
            link: None,
        }
    }

    /// The series' labels, as the side's label filter left them.
    pub fn with_labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    /// The latest value read.
    pub fn with_latest(mut self, latest: Point) -> Self {
        self.latest = Some(latest);
        self
    }

    /// What the value is measured in, such as `requests per second`.
    pub fn with_unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.to_owned());
        self
    }

    /// Where a person can see more, such as a dashboard: an `https://` or `http://` URL without user information, or
    /// [`Error::Invalid`], quoting none of it, since it could hold a credential.
    pub fn with_link(mut self, link: &str) -> Result<Self, Error> {
        let authority = link
            .strip_prefix("https://")
            .or_else(|| link.strip_prefix("http://"))
            .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or(""))
            .filter(|authority| !authority.is_empty() && !authority.contains('@'))
            .filter(|_| link.chars().all(|character| character.is_ascii_graphic()));
        if authority.is_none() {
            return Err(Error::Invalid(
                "a metric's link must be an http:// or https:// URL without user information"
                    .into(),
            ));
        }
        self.link = Some(link.to_owned());
        Ok(self)
    }

    /// What the agent is told: `kind` `metric`, `datasource`, `query`, `at` for an instant or `range`, with `start`,
    /// `end` and `step_seconds`, its times in UTC as RFC 3339, and `labels`, `latest`, with its `value` and `at`,
    /// `unit` and `link`, each null when not given. A value JSON cannot hold, such as `NaN`, is written as Prometheus
    /// writes it, as text.
    pub fn to_json(&self) -> Value {
        let (at, range) = match self.window {
            Window::Instant(at) => (json!(at.rfc3339()), Value::Null),
            Window::Range(range) => (
                Value::Null,
                json!({
                    "start": range.start.rfc3339(),
                    "end": range.end.rfc3339(),
                    "step_seconds": range.step.as_secs_f64(),
                }),
            ),
        };
        json!({
            "kind": "metric",
            "datasource": self.datasource,
            "query": self.query,
            "at": at,
            "range": range,
            "labels": self.labels,
            "latest": self.latest.map(|point| json!({ "value": number(point.value), "at": point.at.rfc3339() })),
            "unit": self.unit,
            "link": self.link,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instant_metric_says_when_with_its_labels_and_values_json_cannot_hold() {
        let at = Timestamp::from_millis(1_762_682_400_000);
        let labels = Labels::from([("service".to_owned(), "checkout".to_owned())]);
        let metric = Metric::instant("staging", "up", at).with_labels(labels);
        let json = metric.to_json();
        assert_eq!(json["at"], "2025-11-09T10:00:00.000Z");
        assert_eq!(json["range"], Value::Null);
        assert_eq!(json["labels"], json!({ "service": "checkout" }));
        assert_eq!(json["latest"], Value::Null);
        assert_eq!(json["unit"], Value::Null);
        assert_eq!(json["link"], Value::Null);
        for (value, written) in [
            (f64::NAN, json!("NaN")),
            (f64::INFINITY, json!("+Inf")),
            (f64::NEG_INFINITY, json!("-Inf")),
            (1.5, json!(1.5)),
        ] {
            let json = metric.clone().with_latest(Point { at, value }).to_json();
            assert_eq!(json["latest"]["value"], written);
            assert_eq!(json["latest"]["at"], "2025-11-09T10:00:00.000Z");
        }
    }

    #[test]
    fn a_link_holds_no_credential() {
        let metric = Metric::instant("production", "up", Timestamp::from_millis(0));
        for link in [
            "https://grafana.example/d/abc?from=now-1h",
            "http://localhost:3000",
        ] {
            assert_eq!(
                metric.clone().with_link(link).unwrap().to_json()["link"],
                link
            );
        }
        for link in [
            "https://user:secret@grafana.example/d/abc",
            "ftp://grafana.example",
            "https://",
            "https://grafana.example/a b",
            "grafana.example",
        ] {
            let error = metric.clone().with_link(link).unwrap_err();
            assert!(!error.to_string().contains("secret"), "{link}");
            assert!(matches!(error, Error::Invalid(_)), "{link}");
        }
    }
}
