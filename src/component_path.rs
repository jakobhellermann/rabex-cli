//! Parsing component references like `Root/Child:2@SpriteRenderer:1`.
//!
//! A reference points at a GameObject by its Transform-hierarchy path and,
//! optionally, one of its components. Neither path segments nor components are
//! unique, so any field may carry a `:<index>` to disambiguate among equally
//! matching siblings/components (0-based). The structural characters `/`, `@`,
//! `:` and `\` can be escaped with a backslash to use them literally in a name.
//!
//! Grammar:
//! ```text
//! path     := segment ('/' segment)* ('@' selector)?
//! segment  := name (':' index)?
//! selector := name (':' index)?
//! ```

use std::fmt;

/// A reference to a GameObject (by hierarchy path) and optionally a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentPath {
    /// Hierarchy path, root first; always at least one segment.
    pub segments: Vec<Field>,
    /// Component selector (`@Type[:index]`), if any.
    pub component: Option<Field>,
}

/// A name plus an optional index disambiguating among equal-named matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub index: Option<usize>,
}

/// How an object is addressed on the command line: by raw path id, or by a
/// hierarchy/component path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectRef {
    PathId(i64),
    Path(ComponentPath),
}

/// Parse an [`ObjectRef`]: a bare integer is a path id, anything else a
/// [`ComponentPath`]. Shaped for use as a clap `value_parser`.
pub fn parse_object_ref(input: &str) -> Result<ObjectRef, String> {
    match input.parse::<i64>() {
        Ok(path_id) => Ok(ObjectRef::PathId(path_id)),
        Err(_) => Ok(ObjectRef::Path(parse(input)?)),
    }
}

/// `Display` is the inverse of [`parse`]: it escapes structural characters so
/// the output round-trips back through `parse`.
impl fmt::Display for ComponentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{segment}")?;
        }
        if let Some(component) = &self.component {
            write!(f, "@{component}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&escape(&self.name))?;
        if let Some(index) = self.index {
            write!(f, ":{index}")?;
        }
        Ok(())
    }
}

/// Backslash-escape the structural characters so a name round-trips.
fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '\\' | '/' | '@' | ':') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Parse a [`ComponentPath`]. Shaped as `fn(&str) -> Result<_, String>` so it
/// can be used directly as a clap `value_parser`.
pub fn parse(input: &str) -> Result<ComponentPath, String> {
    // The component selector is everything after the first unescaped '@'.
    let at = split_keep_escapes(input, '@');
    let (path_part, component) = match at.as_slice() {
        [path] => (path.as_str(), None),
        [path, selector] => (path.as_str(), Some(parse_field(selector, "component")?)),
        _ => return Err("at most one '@' component selector is allowed".to_owned()),
    };

    let segments = split_keep_escapes(path_part, '/')
        .iter()
        .map(|seg| parse_field(seg, "path segment"))
        .collect::<Result<Vec<_>, _>>()?;

    if segments.iter().any(|s| s.name.is_empty()) {
        return Err("empty path segment".to_owned());
    }
    Ok(ComponentPath {
        segments,
        component,
    })
}

fn parse_field(raw: &str, what: &str) -> Result<Field, String> {
    let parts = split_keep_escapes(raw, ':');
    match parts.as_slice() {
        [name] => Ok(Field {
            name: unescape(name),
            index: None,
        }),
        [name, index] => {
            let index = index
                .parse::<usize>()
                .map_err(|_| format!("invalid index ':{index}' (expected a number)"))?;
            Ok(Field {
                name: unescape(name),
                index: Some(index),
            })
        }
        _ => Err(format!("at most one ':index' is allowed per {what}")),
    }
}

/// Split on unescaped `delim`, leaving any other `\x` escapes intact in the
/// pieces (so a later split on a different delimiter still sees them escaped).
fn split_keep_escapes(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            cur.push('\\');
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if c == delim {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    parts.push(cur);
    parts
}

/// Remove escaping backslashes, yielding the literal name.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, index: Option<usize>) -> Field {
        Field {
            name: name.to_owned(),
            index,
        }
    }

    #[test]
    fn plain_path_no_component() {
        assert_eq!(
            parse("Root/Child").unwrap(),
            ComponentPath {
                segments: vec![field("Root", None), field("Child", None)],
                component: None,
            }
        );
    }

    #[test]
    fn path_with_component() {
        assert_eq!(
            parse("Object/Path@SpriteRenderer").unwrap(),
            ComponentPath {
                segments: vec![field("Object", None), field("Path", None)],
                component: Some(field("SpriteRenderer", None)),
            }
        );
    }

    #[test]
    fn indices_on_segments_and_component() {
        assert_eq!(
            parse("Path/To:3/Component@FsmStateMachine:6").unwrap(),
            ComponentPath {
                segments: vec![
                    field("Path", None),
                    field("To", Some(3)),
                    field("Component", None),
                ],
                component: Some(field("FsmStateMachine", Some(6))),
            }
        );
    }

    #[test]
    fn escaped_colon_in_name() {
        assert_eq!(
            parse(r"weird\:name@Comp").unwrap(),
            ComponentPath {
                segments: vec![field("weird:name", None)],
                component: Some(field("Comp", None)),
            }
        );
    }

    #[test]
    fn escaped_slash_and_at_in_name() {
        assert_eq!(
            parse(r"a\/b\@c").unwrap(),
            ComponentPath {
                segments: vec![field("a/b@c", None)],
                component: None,
            }
        );
    }

    #[test]
    fn display_roundtrips_through_parse() {
        for s in [
            "Root/Child",
            "Object/Path@SpriteRenderer",
            "Path/To:3@FsmStateMachine:6",
            r"weird\:name@Comp",
            r"a\/b\@c",
        ] {
            assert_eq!(parse(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn errors() {
        assert!(parse("").is_err());
        assert!(parse("a//b").is_err());
        assert!(parse("a@b@c").is_err());
        assert!(parse("a:notanumber").is_err());
        assert!(parse("a:1:2").is_err());
    }
}
