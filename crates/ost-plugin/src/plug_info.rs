// SPDX-License-Identifier: Apache-2.0
//! Parsing USD `plugInfo.json`.
//!
//! USD's plugin metadata is **JSON-with-comments**, not strict JSON: the `js`
//! library that USD loads it with accepts `#` and `//` line comments, `/* */`
//! block comments, and trailing commas. `usdGenSchema` even writes a leading `#`
//! banner into every file it generates, and the bundled Pixar `plugInfo.json`
//! files carry the same. A strict `serde_json` parse would reject these
//! perfectly valid files, so we strip comments and trailing commas (respecting
//! string literals) before parsing.

/// Parse a USD `plugInfo.json` (JSON-with-comments) into a JSON value.
pub(crate) fn parse_plug_info(src: &str) -> serde_json::Result<serde_json::Value> {
    serde_json::from_str(&strip_jsonc(src))
}

/// Merge the schema `Info.Types` produced by `usdGenSchema` into an existing
/// (co-hosting) `plugInfo.json`, preserving its other entries — e.g. the
/// `SdfFileFormat` type of a file-format plugin co-locating a schema. USD allows
/// one `plugInfo` to register both, and `usdGenSchema` *overwrites* the whole
/// file, so a co-hosting build must merge rather than clobber.
///
/// The generated schema types are inserted into the target's first plugin
/// entry's `Info.Types` (creating `Info`/`Types` if absent). Any schema types
/// already in the target from a previous merge (identified by their
/// `schemaKind` marker) are first dropped, so a renamed or removed type does
/// not linger as a stale entry across rebuilds; the co-host's own types (e.g.
/// an `SdfFileFormat` entry, which carries no `schemaKind`) are preserved.
/// Returns the merged plugInfo as pretty JSON. Errors if either side is not a
/// `plugInfo`-shaped document (no `Plugins` array, or an empty one on the
/// target).
pub fn merge_schema_types(target_src: &str, generated_src: &str) -> Result<String, MergeError> {
    let mut target = parse_plug_info(target_src).map_err(|_| MergeError::Parse("target"))?;
    let generated = parse_plug_info(generated_src).map_err(|_| MergeError::Parse("generated"))?;

    // Collect every schema type the generated plugInfo declares.
    let mut schema_types = serde_json::Map::new();
    for plugin in generated
        .get("Plugins")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(types) = plugin.pointer("/Info/Types").and_then(|t| t.as_object()) {
            for (k, v) in types {
                schema_types.insert(k.clone(), v.clone());
            }
        }
    }
    if schema_types.is_empty() {
        return Err(MergeError::NoGeneratedTypes);
    }

    // Insert them into the target's first plugin entry's `Info.Types`.
    let plugins = target
        .get_mut("Plugins")
        .and_then(|p| p.as_array_mut())
        .ok_or(MergeError::NoPlugins)?;
    let entry = plugins.first_mut().ok_or(MergeError::NoPlugins)?;
    let entry = entry.as_object_mut().ok_or(MergeError::NoPlugins)?;
    let info = entry
        .entry("Info")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or(MergeError::NoPlugins)?;
    let types = info
        .entry("Types")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or(MergeError::NoPlugins)?;
    // Drop any previously-merged schema types (they carry usdGenSchema's
    // `schemaKind` marker) so a rename or removal in `schema.usda` doesn't leave
    // a stale entry — the co-host's own types (no `schemaKind`) stay.
    types.retain(|_, v| v.get("schemaKind").is_none());
    for (k, v) in schema_types {
        types.insert(k, v);
    }

    serde_json::to_string_pretty(&target).map_err(|_| MergeError::Parse("serialize"))
}

/// Why a [`merge_schema_types`] call could not produce a merged plugInfo.
#[derive(Debug, PartialEq, Eq)]
pub enum MergeError {
    /// A side was not valid plugInfo JSON (which side).
    Parse(&'static str),
    /// The generated plugInfo declared no schema `Info.Types`.
    NoGeneratedTypes,
    /// The target has no `Plugins` array / entry to merge into.
    NoPlugins,
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::Parse(side) => write!(f, "{side} plugInfo.json is not valid JSON"),
            MergeError::NoGeneratedTypes => {
                write!(f, "usdGenSchema output declared no schema Types to merge")
            }
            MergeError::NoPlugins => {
                write!(f, "target plugInfo.json has no Plugins entry to merge into")
            }
        }
    }
}

/// Strip `#`/`//` line comments, `/* */` block comments, and trailing commas
/// from `src`, leaving string literals untouched.
fn strip_jsonc(src: &str) -> String {
    let decommented = strip_comments(src);
    remove_trailing_commas(&decommented)
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_str = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push('"');
            }
            '#' => skip_to_eol(&mut chars),
            '/' if chars.peek() == Some(&'/') => skip_to_eol(&mut chars),
            '/' if chars.peek() == Some(&'*') => {
                chars.next(); // consume '*'
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn skip_to_eol(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&n) = chars.peek() {
        if n == '\n' {
            break;
        }
        chars.next();
    }
}

/// Drop a `,` that is followed (after whitespace) by `}` or `]`, outside strings.
fn remove_trailing_commas(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut in_str = false;
    let mut escaped = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            let next = chars[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue; // trailing comma — drop it
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usdgenschema_style_banner_and_comments() {
        // usdGenSchema writes a leading `#` banner; USD accepts it.
        let src = r#"# Portions of this file auto-generated by usdGenSchema.
# Edits will survive regeneration except for comments.
{
    "Plugins": [
        { "Type": "resource", "Name": "x" } // trailing line comment
    ]
}"#;
        let v = parse_plug_info(src).expect("parses JSONC");
        assert_eq!(v["Plugins"][0]["Name"], "x");
    }

    #[test]
    fn handles_block_comments_and_trailing_commas() {
        let src = r#"{
    /* block
       comment */
    "Plugins": [
        { "Name": "a", },
        { "Name": "b", }
    ],
}"#;
        let v = parse_plug_info(src).expect("parses");
        assert_eq!(v["Plugins"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn leaves_comment_like_and_comma_chars_inside_strings() {
        // `#`, `//`, and `,]` inside string values must be preserved.
        let src = r#"{ "LibraryPath": "../a//b#c", "note": "x,]y" }"#;
        let v = parse_plug_info(src).expect("parses");
        assert_eq!(v["LibraryPath"], "../a//b#c");
        assert_eq!(v["note"], "x,]y");
    }

    #[test]
    fn still_rejects_genuinely_invalid_json() {
        assert!(parse_plug_info("{ not json").is_err());
    }

    #[test]
    fn merge_adds_schema_types_and_keeps_the_fileformat_entry() {
        // A co-hosting file-format plugInfo: one library entry with its own type.
        let target = r#"{
            "Plugins": [
                {
                    "Type": "library",
                    "Name": "toy",
                    "LibraryPath": "../../../lib/libToy.dll",
                    "Info": { "Types": { "ToyFileFormat": { "bases": ["SdfFileFormat"] } } }
                }
            ]
        }"#;
        // usdGenSchema output (JSONC banner) declaring the schema type.
        let generated = r#"# auto-generated by usdGenSchema
        {
            "Plugins": [
                { "Type": "resource", "Name": "toy",
                  "Info": { "Types": { "ToyVrmAPI": { "schemaIdentifier": "VrmAPI", "schemaKind": "singleApplyAPI", "bases": ["UsdAPISchemaBase"] } } } }
            ]
        }"#;
        let merged = merge_schema_types(target, generated).expect("merges");
        let v = parse_plug_info(&merged).unwrap();
        let types = v["Plugins"][0]["Info"]["Types"].as_object().unwrap();
        // Both the file-format type and the merged schema type are present.
        assert!(types.contains_key("ToyFileFormat"));
        assert!(types.contains_key("ToyVrmAPI"));
        // The library entry is preserved.
        assert_eq!(v["Plugins"][0]["LibraryPath"], "../../../lib/libToy.dll");
    }

    #[test]
    fn merge_prunes_stale_schema_types_but_keeps_the_cohost_type() {
        // A target that already carries a previously-merged schema type
        // (`ToyOldAPI`, with a `schemaKind` marker) plus the file-format type.
        let target = r#"{
            "Plugins": [
                {
                    "Type": "library",
                    "Name": "toy",
                    "Info": { "Types": {
                        "ToyFileFormat": { "bases": ["SdfFileFormat"] },
                        "ToyOldAPI": { "schemaIdentifier": "OldAPI", "schemaKind": "singleApplyAPI", "bases": ["UsdAPISchemaBase"] }
                    } }
                }
            ]
        }"#;
        // A fresh regenerate that renamed the schema type OldAPI -> NewAPI.
        let generated = r#"{
            "Plugins": [
                { "Info": { "Types": { "ToyNewAPI": { "schemaIdentifier": "NewAPI", "schemaKind": "singleApplyAPI", "bases": ["UsdAPISchemaBase"] } } } }
            ]
        }"#;
        let merged = merge_schema_types(target, generated).expect("merges");
        let v = parse_plug_info(&merged).unwrap();
        let types = v["Plugins"][0]["Info"]["Types"].as_object().unwrap();
        // The stale schema type is gone, the new one is present, and the
        // co-host's own (schemaKind-less) file-format type survives.
        assert!(!types.contains_key("ToyOldAPI"));
        assert!(types.contains_key("ToyNewAPI"));
        assert!(types.contains_key("ToyFileFormat"));
    }

    #[test]
    fn merge_errors_when_generated_has_no_types_or_target_is_empty() {
        let target = r#"{ "Plugins": [ { "Name": "x", "Info": {} } ] }"#;
        assert_eq!(
            merge_schema_types(target, r#"{ "Plugins": [] }"#),
            Err(MergeError::NoGeneratedTypes)
        );
        let generated = r#"{ "Plugins": [ { "Info": { "Types": { "A": {} } } } ] }"#;
        assert_eq!(
            merge_schema_types(r#"{ "Plugins": [] }"#, generated),
            Err(MergeError::NoPlugins)
        );
    }
}
