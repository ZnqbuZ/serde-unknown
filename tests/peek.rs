#![cfg(feature = "peek")]

use serde::{Deserialize, de};
use serde_unknown::DeserializerExt;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
}

#[test]
fn filter_selects_complete_values_with_nested_paths() {
    let json = r#"[
        {"name":"first","extra":{"deep":[1,{"key":true}]},"skip":1e400},
        {"name":"second","extra":null}
    ]"#;
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let mut paths = Vec::new();
    let mut values = Vec::new();
    let entries: Vec<Entry> = (&mut deserializer)
        .track()
        .filter(|path| {
            paths.push(path.to_string());
            Ok(if path.matches(&["<root>", "0", "extra"]) {
                Some(0usize)
            } else if path.matches(&["<root>", "1", "extra"]) {
                Some(1)
            } else {
                None
            })
        })
        .peek(|path, index, value| {
            values.push((index, path.to_string(), value));
            Ok(())
        })
        .deserialize()
        .unwrap();

    assert_eq!(entries[0].name, "first");
    assert_eq!(entries[1].name, "second");
    assert_eq!(paths, ["[0].extra", "[0].skip", "[1].extra"]);
    assert_eq!(
        values,
        [
            (
                Some(0),
                "[0].extra".to_owned(),
                serde_json::json!({"deep": [1, {"key": true}]}),
            ),
            (Some(1), "[1].extra".to_owned(), serde_json::Value::Null),
        ]
    );
    deserializer.end().unwrap();
}

#[test]
fn peek_without_filter_receives_every_unknown_value() {
    let mut deserializer = serde_json::Deserializer::from_str(
        r#"{"name":"a","boolean":true,"number":7,"string":"text","array":[1,2]}"#,
    );
    let mut values = Vec::new();
    let entry: Entry = (&mut deserializer)
        .track()
        .peek(|path, context, value| {
            values.push((context, path.to_string(), value));
            Ok(())
        })
        .deserialize()
        .unwrap();

    assert_eq!(entry.name, "a");
    assert_eq!(
        values,
        [
            (None, "boolean".to_owned(), serde_json::json!(true)),
            (None, "number".to_owned(), serde_json::json!(7)),
            (None, "string".to_owned(), serde_json::json!("text")),
            (None, "array".to_owned(), serde_json::json!([1, 2])),
        ]
    );
    deserializer.end().unwrap();
}

#[test]
fn filter_can_be_configured_after_peek() {
    let mut deserializer =
        serde_json::Deserializer::from_str(r#"{"name":"a","skip":1e400,"extra":42}"#);
    let mut values = Vec::new();
    let entry: Entry = (&mut deserializer)
        .track()
        .peek(|path, context, value| {
            values.push((context, path.to_string(), value));
            Ok(())
        })
        .filter(|path| Ok(path.matches(&["<root>", "extra"]).then_some("extra")))
        .deserialize()
        .unwrap();

    assert_eq!(entry.name, "a");
    assert_eq!(
        values,
        [(Some("extra"), "extra".to_owned(), serde_json::json!(42))]
    );
    deserializer.end().unwrap();
}

#[test]
fn value_errors_propagate_without_calling_peek() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"name":"a","extra":1e400}"#);
    let mut filters = 0;
    let mut peeks = 0;
    let result: Result<Entry, _> = (&mut deserializer)
        .track()
        .filter(|_| {
            filters += 1;
            Ok(Some(()))
        })
        .peek(|_, _, _| {
            peeks += 1;
            Ok(())
        })
        .deserialize();

    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("number out of range")
    );
    assert_eq!(filters, 1);
    assert_eq!(peeks, 0);
}

#[test]
fn peek_uses_the_serde_data_model() {
    #[derive(Debug, Deserialize)]
    struct Number {
        value: u8,
    }

    let deserializer = de::value::MapDeserializer::<_, de::value::Error>::new(
        [("value", 3u8), ("extra", 7u8)].into_iter(),
    );
    let mut values = Vec::new();
    let number: Number = deserializer
        .track()
        .peek(|path, _, value| {
            values.push((path.to_string(), value));
            Ok(())
        })
        .deserialize()
        .unwrap();

    assert_eq!(number.value, 3);
    assert_eq!(values, [("extra".to_owned(), serde_json::json!(7))]);
}

#[test]
fn peek_errors_stop_before_later_values() {
    let mut deserializer = serde_json::Deserializer::from_str(
        r#"{"name":"a","first":{"nested":true},"second":invalid}"#,
    );
    let mut peeks = 0;
    let result: Result<Entry, _> = (&mut deserializer)
        .track()
        .peek(|path, _, value| {
            peeks += 1;
            assert!(path.matches(&["<root>", "first"]));
            assert_eq!(value, serde_json::json!({"nested": true}));
            Err(de::Error::custom("blocked value"))
        })
        .deserialize();

    assert_eq!(result.unwrap_err().to_string(), "blocked value");
    assert_eq!(peeks, 1);
}
