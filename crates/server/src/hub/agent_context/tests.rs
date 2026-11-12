// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn instants_are_written_in_utc_and_read_only_with_their_offset() {
    assert_eq!(instant(1_794_450_000_000), "2026-11-12T02:20:00Z");
    // Beyond what a time stamp may be, the milliseconds themselves.
    assert_eq!(instant(i64::MAX), i64::MAX.to_string());
    assert_eq!(
        parse_instant(" 2026-11-12T02:20:00Z ", "from"),
        Ok(1_794_450_000_000)
    );
    assert_eq!(
        parse_instant("2026-11-12T13:20:00+11:00", "from"),
        Ok(1_794_450_000_000)
    );
    // Past the year 9999, it says so, however many digits it is written with; an extended year within it is read.
    for late in ["10000-01-01T00:00:00Z", "+010000-01-01T00:00:00Z"] {
        assert_eq!(
            parse_instant(late, "to"),
            Err(
                "to is past the year 9999: a time window is within the years 0 to 9999.".to_owned()
            )
        );
    }
    assert_eq!(
        parse_instant("+002026-11-12T02:20:00Z", "from"),
        Ok(1_794_450_000_000)
    );
    for local in ["2026-11-12T02:20:00", "02:20", "yesterday"] {
        assert!(
            parse_instant(local, "to")
                .unwrap_err()
                .starts_with("to is an instant with its offset from UTC"),
            "{local}"
        );
    }
}

#[test]
fn settings_and_chips_are_told_as_the_agent_reads_them() {
    let fixed = Setting::Fixed {
        value: "EU-WEST1".into(),
    };
    let window = Setting::Window {
        from_ms: 1_794_450_000_000,
        to_ms: 1_794_451_800_000,
    };
    assert_eq!(
        setting_json("region", &fixed),
        json!({"parameter": "region", "value": "EU-WEST1"})
    );
    assert_eq!(
        setting_json("time", &Setting::Live { minutes: 30 }),
        json!({"parameter": "time", "live_minutes": 30})
    );
    assert_eq!(
        setting_json("time", &window),
        json!({"parameter": "time", "from": "2026-11-12T02:20:00Z", "to": "2026-11-12T02:50:00Z"})
    );
    let chip = |value: ContextValue, setting: ContextSetting| {
        chip_json(&ContextChip::new("time", "Time", value, setting))
    };
    assert_eq!(
        chip(ContextValue::Live { minutes: 15 }, ContextSetting::Follows),
        json!({"parameter": "time", "label": "Time", "live_minutes": 15, "setting": "follows"})
    );
    assert_eq!(
        chip(
            ContextValue::Window {
                from_ms: 0,
                to_ms: 60_000
            },
            ContextSetting::Inherited
        ),
        json!({"parameter": "time", "label": "Time", "from": "1970-01-01T00:00:00Z", "to": "1970-01-01T00:01:00Z", "setting": "inherited"})
    );
    // A value of a kind this server does not know is told as its label alone.
    assert_eq!(
        chip(ContextValue::Unknown, ContextSetting::Fixed),
        json!({"parameter": "time", "label": "Time", "setting": "fixed"})
    );
}
