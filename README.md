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
        if path.matches(&["<root>", "future"]) {
            eprintln!("Unknown field: {path}");
        }
        Ok(())
    })?;
    deserializer.end()?;
    assert_eq!(config.port, 8080);
    Ok(())
}
```

The callback returns `Result<(), D::Error>`. Return `Ok(())` to continue, or
return the deserializer's native error to reject the input:

```rust
use serde::{Deserialize, de::Error};
use serde_unknown::DeserializerExt;

#[derive(Deserialize)]
struct Config {
    port: u16,
}

let mut deserializer = serde_json::Deserializer::from_str(
    r#"{"port":8080,"forbidden":{"nested":true}}"#,
);
let result: Result<Config, _> = (&mut deserializer).deserialize(|path| {
    if path.matches(&["<root>", "forbidden"]) {
        return Err(Error::custom("forbidden field"));
    }
    Ok(())
});
assert_eq!(result.err().unwrap().to_string(), "forbidden field");
```

The callback runs before the adapter delegates consumption of the unknown value.
The first callback error is saved and returned unchanged from the outermost call,
even if a nested deserializer uses a different error type. Nested code can catch
the intermediate error and continue executing, but the outermost call still
returns the saved error. Subsequent callbacks are not invoked after rejection.
Saving the error requires no heap allocation. Intermediate deserializers may
construct errors to stop processing, and error construction can allocate.

Paths use a stack-linked list. Keys use `CompactString`, so short keys stay inline
while long keys may allocate. Invalid values and duplicate fields still return
errors. Map keys are unrestricted. Flattened fields are not tracked.

`Path::matches` compares the complete path without formatting or allocation.
Patterns run from root to leaf and start with `"<root>"`. `"*"` matches one key
or sequence index; indices use decimal numbers, such as `"3"`. For example,
`path.matches(&["<root>", "config", "*", "deprecated_field"])` matches the field
under one entry in `config`. Keys containing dots or brackets remain one segment.
Matching takes linear time in the compared segments and constant auxiliary space.

## Features

The `std` feature is enabled by default. Disable default features to use
`serde-unknown` in a `no_std` environment:

```toml
serde-unknown = { version = "0.1", default-features = false }
```

This configuration still requires `alloc` and an allocator in the final
application. Short keys stay inline; long keys may allocate. `Path` implements
`Display` and `Debug` with or without `std`.
