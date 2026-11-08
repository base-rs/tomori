// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Prometheus's answers, typed, and how they are read from its JSON.
use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Scrub};
use crate::{LabelFilter, Timestamp};

/// A series' labels, by name, after the client's [`LabelFilter`].
pub type Labels = BTreeMap<String, String>;

/// One value at its moment. Prometheus's `NaN`, `+Inf` and `-Inf` read as `f64`'s; a stale marker reads as `NaN`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at: Timestamp,
    pub value: f64,
}

/// A series' value at the moment an instant query was evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub metric: Labels,
    pub point: Point,
}

/// A series' values over a range, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub metric: Labels,
    pub points: Vec<Point>,
}

/// What an instant query evaluated to.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// A sample of each series, an instant vector.
    Vector(Vec<Sample>),
    /// A range of each series, as from a range selector such as `up[5m]`.
    Matrix(Vec<Series>),
    /// One number.
    Scalar(Point),
    /// One string, at its moment.
    String(Timestamp, String),
}

impl Data {
    /// The samples, if this is an instant vector, as most instant queries answer.
    pub fn into_vector(self) -> Option<Vec<Sample>> {
        match self {
            Self::Vector(samples) => Some(samples),
            _ => None,
        }
    }
}

/// What a query answered, with the warnings Prometheus gave beside it, such as about results it may have missed, scrubbed as errors are (see [`Error`]).
///
/// Samples of native histograms, which this crate does not read, are left out, keeping a series' float samples, and a warning says how many: wrap them in `histogram_count()`, `histogram_sum()` or `histogram_quantile()` to read them as floats.
#[derive(Clone, Debug, PartialEq)]
pub struct Response<T> {
    pub data: T,
    pub warnings: Vec<String>,
}

/// The parts of an answer that matter for reading it.
pub(crate) struct Answer<'a> {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: &'a [u8],
}

/// Read an answer: Prometheus's error, a failed status, or its data, keeping the labels `filter` keeps. Whatever is quoted is scrubbed with `scrub`.
pub(crate) fn read(
    answer: Answer<'_>,
    filter: &LabelFilter,
    scrub: &Scrub<'_>,
) -> Result<Response<Data>, Error> {
    let json: Option<Value> = serde_json::from_slice(answer.body).ok();
    let retry_after = answer.retry_after.map(Duration::from_secs);
    if let Some(json) = &json
        && json["status"] == "error"
    {
        return Err(Error::Query {
            status: answer.status,
            retry_after,
            error_type: scrub.quote(json["errorType"].as_str().unwrap_or("")),
            message: scrub.quote(json["error"].as_str().unwrap_or("")),
        });
    }
    if !(200..300).contains(&answer.status) {
        return Err(Error::Status {
            status: answer.status,
            retry_after,
            detail: scrub.quote(&String::from_utf8_lossy(answer.body)),
        });
    }
    let json = json.ok_or(unreadable("it is not JSON"))?;
    if json["status"] != "success" {
        return Err(unreadable("it does not say it succeeded"));
    }
    let mut warnings = Vec::new();
    for warning in json["warnings"].as_array().into_iter().flatten() {
        warnings.push(scrub.quote(warning.as_str().unwrap_or("")));
    }
    let (data, skipped) = data(&json["data"], filter)?;
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} native histogram samples were left out; wrap them in histogram_count(), histogram_sum() or histogram_quantile() to read them"
        ));
    }
    Ok(Response { data, warnings })
}

fn unreadable(why: &str) -> Error {
    Error::Parse(why.to_owned())
}

/// The data, and how many samples of native histograms were left out of it.
fn data(data: &Value, filter: &LabelFilter) -> Result<(Data, usize), Error> {
    let result = &data["result"];
    match data["resultType"].as_str() {
        Some("vector") => {
            let (mut samples, mut skipped) = (Vec::new(), 0);
            for row in rows(result)? {
                if row.get("histogram").is_some() {
                    skipped += 1;
                    continue;
                }
                samples.push(sample(row, filter)?);
            }
            Ok((Data::Vector(samples), skipped))
        }
        Some("matrix") => {
            let (mut matrix, mut skipped) = (Vec::new(), 0);
            for row in rows(result)? {
                // A series may mix float samples with histograms: its floats are kept.
                let histograms = row["histograms"].as_array().map_or(0, Vec::len);
                skipped += histograms;
                if histograms > 0 && row.get("values").is_none() {
                    continue;
                }
                matrix.push(series(row, filter)?);
            }
            Ok((Data::Matrix(matrix), skipped))
        }
        Some("scalar") => Ok((Data::Scalar(point(result)?), 0)),
        Some("string") => {
            let at = timestamp(&result[0])?;
            let text = result[1]
                .as_str()
                .ok_or(unreadable("a string is not text"))?;
            Ok((Data::String(at, text.to_owned()), 0))
        }
        _ => Err(unreadable("its result type is not one Prometheus has")),
    }
}

fn rows(result: &Value) -> Result<&Vec<Value>, Error> {
    result
        .as_array()
        .ok_or(unreadable("its result is not a list"))
}

fn sample(row: &Value, filter: &LabelFilter) -> Result<Sample, Error> {
    Ok(Sample {
        metric: labels(&row["metric"], filter)?,
        point: point(&row["value"])?,
    })
}

fn series(row: &Value, filter: &LabelFilter) -> Result<Series, Error> {
    let values = row["values"]
        .as_array()
        .ok_or(unreadable("a series has no values"))?;
    Ok(Series {
        metric: labels(&row["metric"], filter)?,
        points: values.iter().map(point).collect::<Result<_, _>>()?,
    })
}

fn labels(metric: &Value, filter: &LabelFilter) -> Result<Labels, Error> {
    let metric = metric
        .as_object()
        .ok_or(unreadable("a series has no labels"))?;
    let mut labels = Labels::new();
    for (name, value) in metric {
        if filter.keeps(name) {
            let value = value.as_str().ok_or(unreadable("a label is not text"))?;
            labels.insert(name.clone(), value.to_owned());
        }
    }
    Ok(labels)
}

fn timestamp(value: &Value) -> Result<Timestamp, Error> {
    value
        .as_f64()
        .and_then(Timestamp::from_seconds)
        .ok_or(unreadable("a timestamp is not a number"))
}

fn point(value: &Value) -> Result<Point, Error> {
    Ok(Point {
        at: timestamp(&value[0])?,
        value: value[1]
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or(unreadable("a value is not a number in a string"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(body: Value) -> Result<Response<Data>, Error> {
        read_status(200, body)
    }

    fn read_status(status: u16, body: Value) -> Result<Response<Data>, Error> {
        let body = body.to_string();
        read(
            Answer {
                status,
                retry_after: None,
                body: body.as_bytes(),
            },
            &LabelFilter::all(),
            &SCRUB,
        )
    }

    const SCRUB: Scrub<'static> = Scrub {
        token: Some("s3cret"),
        host: "observe.example",
        base: "",
    };

    fn success(data: Value) -> Value {
        json!({"status": "success", "data": data})
    }

    #[test]
    fn vectors_keep_their_labels_times_and_special_values() {
        let response = ok(success(json!({"resultType": "vector", "result": [
            {"metric": {"__name__": "up", "pod": "a"}, "value": [1700000000.5, "1"]},
            {"metric": {}, "value": [1700000000, "NaN"]},
            {"metric": {}, "value": [1700000000, "+Inf"]},
            {"metric": {}, "value": [1700000000, "-Inf"]},
            {"metric": {}, "value": [1700000000, "1.5e3"]},
        ]})))
        .unwrap();
        assert!(response.warnings.is_empty());
        let samples = response.data.into_vector().unwrap();
        assert_eq!(samples[0].metric["pod"], "a");
        assert_eq!(samples[0].metric["__name__"], "up");
        assert_eq!(
            samples[0].point.at,
            Timestamp::from_millis(1_700_000_000_500)
        );
        assert_eq!(samples[0].point.value, 1.0);
        assert!(samples[1].point.value.is_nan());
        assert_eq!(samples[2].point.value, f64::INFINITY);
        assert_eq!(samples[3].point.value, f64::NEG_INFINITY);
        assert_eq!(samples[4].point.value, 1500.0);
    }

    #[test]
    fn matrices_scalars_and_strings_are_read() {
        let matrix = ok(success(json!({"resultType": "matrix", "result": [
            {"metric": {"job": "web"}, "values": [[1, "1"], [2, "2"]]},
        ]})))
        .unwrap();
        assert_eq!(
            matrix.data,
            Data::Matrix(vec![Series {
                metric: Labels::from([("job".into(), "web".into())]),
                points: vec![
                    Point {
                        at: Timestamp::from_millis(1000),
                        value: 1.0
                    },
                    Point {
                        at: Timestamp::from_millis(2000),
                        value: 2.0
                    },
                ],
            }])
        );
        assert_eq!(matrix.data.into_vector(), None);
        let scalar = ok(success(json!({"resultType": "scalar", "result": [3, "4"]}))).unwrap();
        assert_eq!(
            scalar.data,
            Data::Scalar(Point {
                at: Timestamp::from_millis(3000),
                value: 4.0
            })
        );
        let string = ok(success(
            json!({"resultType": "string", "result": [3, "hello"]}),
        ))
        .unwrap();
        assert_eq!(
            string.data,
            Data::String(Timestamp::from_millis(3000), "hello".into())
        );
    }

    #[test]
    fn warnings_are_kept_without_the_token() {
        let mut body = success(json!({"resultType": "vector", "result": []}));
        body["warnings"] = json!(["partial s3cret", 7]);
        assert_eq!(ok(body).unwrap().warnings, ["partial [redacted]", ""]);
    }

    #[test]
    fn labels_are_filtered_before_they_are_returned() {
        let body = success(json!({"resultType": "vector", "result": [
            {"metric": {"__name__": "up", "pod": "a", "url": "https://user:pw@x", "odd": 7}, "value": [1, "1"]},
        ]}))
        .to_string();
        let answer = Answer {
            status: 200,
            retry_after: None,
            body: body.as_bytes(),
        };
        let response = read(answer, &LabelFilter::only(["pod"]), &SCRUB).unwrap();
        let samples = response.data.into_vector().unwrap();
        assert_eq!(
            samples[0].metric,
            Labels::from([("pod".into(), "a".into())])
        );
    }

    #[test]
    fn prometheus_errors_and_failed_statuses_are_typed() {
        let refused =
            json!({"status": "error", "errorType": "bad_data", "error": "parse error near s3cret"});
        assert_eq!(
            read_status(400, refused).unwrap_err(),
            Error::Query {
                status: 400,
                retry_after: None,
                error_type: "bad_data".into(),
                message: "parse error near [redacted]".into()
            }
        );
        assert_eq!(
            read_status(422, json!({"status": "error"})).unwrap_err(),
            Error::Query {
                status: 422,
                retry_after: None,
                error_type: String::new(),
                message: String::new()
            }
        );
        let wait = Some(Duration::from_secs(30));
        // Prometheus's own 503, and a gateway's 429, in Prometheus's form, keep their status and retry-after.
        for (status, error_type) in [(503, "unavailable"), (429, "too_many_requests")] {
            let body = json!({"status": "error", "errorType": error_type, "error": "busy at observe.example"})
                .to_string();
            let answer = Answer {
                status,
                retry_after: Some(30),
                body: body.as_bytes(),
            };
            assert_eq!(
                read(answer, &LabelFilter::all(), &SCRUB).unwrap_err(),
                Error::Query {
                    status,
                    retry_after: wait,
                    error_type: error_type.into(),
                    message: "busy at <endpoint>".into(),
                }
            );
        }
        let body = b"<html>busy s3cret at https://observe.example/</html>";
        let error = read(
            Answer {
                status: 503,
                retry_after: Some(30),
                body,
            },
            &LabelFilter::all(),
            &SCRUB,
        )
        .unwrap_err();
        assert_eq!(
            error,
            Error::Status {
                status: 503,
                retry_after: wait,
                detail: "<html>busy [redacted] at https://<endpoint>/</html>".into(),
            }
        );
    }

    #[test]
    fn native_histograms_are_left_out_with_a_warning() {
        let vector = ok(success(json!({"resultType": "vector", "result": [
            {"metric": {"le": "a"}, "histogram": [1, {"count": "2"}]},
            {"metric": {"pod": "web-1"}, "value": [1, "1"]},
            {"metric": {"le": "b"}, "histogram": [1, {"count": "3"}]},
        ]})))
        .unwrap();
        assert_eq!(
            vector.warnings,
            [
                "2 native histogram samples were left out; wrap them in histogram_count(), histogram_sum() or histogram_quantile() to read them"
            ]
        );
        let samples = vector.data.into_vector().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].metric["pod"], "web-1");
        let matrix = ok(success(json!({"resultType": "matrix", "result": [
            {"metric": {}, "histograms": [[1, {"count": "2"}]]},
            {"metric": {"pod": "web-1"}, "values": [[1, "1"]], "histograms": [[2, {"count": "1"}], [3, {"count": "1"}]]},
        ]})))
        .unwrap();
        assert_eq!(matrix.warnings.len(), 1);
        assert!(matrix.warnings[0].starts_with("3 native histogram samples were left out"));
        assert_eq!(
            matrix.data,
            Data::Matrix(vec![Series {
                metric: Labels::from([("pod".into(), "web-1".into())]),
                points: vec![Point {
                    at: Timestamp::from_millis(1000),
                    value: 1.0
                }],
            }])
        );
    }

    #[test]
    fn what_cannot_be_read_is_refused() {
        let parse = |why: &str| Error::Parse(why.to_owned());
        let answer = Answer {
            status: 200,
            retry_after: None,
            body: b"not json",
        };
        assert_eq!(
            read(answer, &LabelFilter::all(), &SCRUB).unwrap_err(),
            parse("it is not JSON")
        );
        let cases = [
            (json!({"status": "pending"}), "it does not say it succeeded"),
            (
                success(json!({"resultType": "histogram"})),
                "its result type is not one Prometheus has",
            ),
            (
                success(json!({"resultType": "vector", "result": {}})),
                "its result is not a list",
            ),
            (
                success(json!({"resultType": "matrix"})),
                "its result is not a list",
            ),
            (
                success(json!({"resultType": "scalar", "result": ["x", "1"]})),
                "a timestamp is not a number",
            ),
            (
                success(json!({"resultType": "matrix", "result": [{"metric": [], "values": []}]})),
                "a series has no labels",
            ),
            (
                success(json!({"resultType": "matrix", "result": [{"metric": {}}]})),
                "a series has no values",
            ),
            (
                success(
                    json!({"resultType": "matrix", "result": [{"metric": {}, "values": [[1, 2]]}]}),
                ),
                "a value is not a number in a string",
            ),
            (
                success(json!({"resultType": "vector", "result": [{"value": [1, "1"]}]})),
                "a series has no labels",
            ),
            (
                success(
                    json!({"resultType": "vector", "result": [{"metric": {"a": 1}, "value": [1, "1"]}]}),
                ),
                "a label is not text",
            ),
            (
                success(
                    json!({"resultType": "vector", "result": [{"metric": {}, "value": ["1", "1"]}]}),
                ),
                "a timestamp is not a number",
            ),
            (
                success(
                    json!({"resultType": "vector", "result": [{"metric": {}, "value": [1, "one"]}]}),
                ),
                "a value is not a number in a string",
            ),
            (
                success(json!({"resultType": "string", "result": [1, 2]})),
                "a string is not text",
            ),
            (
                success(json!({"resultType": "string", "result": ["x", "y"]})),
                "a timestamp is not a number",
            ),
        ];
        for (body, why) in cases {
            assert_eq!(ok(body).unwrap_err(), parse(why), "{why}");
        }
    }
}
