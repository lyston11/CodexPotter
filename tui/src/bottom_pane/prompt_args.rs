/// Parse a first-line slash command of the form `/name <rest>`.
///
/// Returns `(name, rest_after_name, rest_offset)` if the line begins with `/`
/// and contains a non-empty name; otherwise returns `None`.
///
/// `rest_offset` is the byte index into the original line where `rest_after_name`
/// starts after trimming leading whitespace (so `line[rest_offset..] == rest_after_name`).
pub fn parse_slash_name(line: &str) -> Option<(&str, &str, usize)> {
    let stripped = line.strip_prefix('/')?;
    let mut name_end_in_stripped = stripped.len();
    for (idx, ch) in stripped.char_indices() {
        if ch.is_whitespace() {
            name_end_in_stripped = idx;
            break;
        }
    }
    let name = &stripped[..name_end_in_stripped];
    if name.is_empty() {
        return None;
    }
    let rest_untrimmed = &stripped[name_end_in_stripped..];
    let rest = rest_untrimmed.trim_start();
    let rest_start_in_stripped = name_end_in_stripped + (rest_untrimmed.len() - rest.len());
    // `stripped` is `line` without the leading '/', so add 1 to get the original offset.
    let rest_offset = rest_start_in_stripped + 1;
    Some((name, rest, rest_offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_slash_name_extracts_name_and_rest() {
        let line = "/mention   @foo/bar";
        let (name, rest, rest_offset) = parse_slash_name(line).expect("parse");
        assert_eq!(name, "mention");
        assert_eq!(rest, "@foo/bar");
        assert_eq!(&line[rest_offset..], rest);
    }

    #[test]
    fn parse_slash_name_returns_none_for_missing_name() {
        assert_eq!(parse_slash_name("/"), None);
        assert_eq!(parse_slash_name("not-a-command"), None);
    }
}
