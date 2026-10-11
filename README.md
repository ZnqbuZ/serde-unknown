# serde-unknown

Handle unknown fields during Serde deserialization, including struct
fields that would normally be rejected by `deny_unknown_fields` or pbjson.
The callback receives the field path; the caller chooses how to handle it.
Use `DeserializerExt::deserialize` for method syntax, or the
`serde_unknown::deserialize` function for the same behavior.

```rust
use serde::Deserialize;
use serde_unknown::DeserializerExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    port: u16,
}

fn main() -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(
        r#"{"port":8080,"future":{"nested":true}}"#,
    );
    let config: Config = (&mut deserializer).deserialize(|path| {
        eprintln!("Unknown field: {path}");
    })?;
    deserializer.end()?;
    assert_eq!(config.port, 8080);
    Ok(())
}
```

Paths use a stack-linked list. Keys use `CompactString`, so short keys stay inline
while long keys may allocate. Invalid values and duplicate fields still return
errors. Map keys are unrestricted. Flattened fields are not tracked.
