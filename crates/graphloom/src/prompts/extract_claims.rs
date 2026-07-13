//! Fixed gleaning prompts for claim extraction.

/// Requests another claim extraction pass.
pub(crate) const CONTINUE_PROMPT: &str = concat!(
    "MANY entities were missed in the last extraction.  ",
    "Add them below using the same format:\n",
);

/// Asks whether claim extraction requires another gleaning pass.
pub(crate) const LOOP_PROMPT: &str = concat!(
    "It appears some entities may have still been missed. Answer Y if there are still entities ",
    "that need to be added, or N if there are none. Please answer with a single letter Y or N.\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_match_graphrag_gleaning_prompts_byte_for_byte() {
        assert_eq!(CONTINUE_PROMPT.as_bytes(), b"MANY entities were missed in the last extraction.  Add them below using the same format:\n");
        assert!(CONTINUE_PROMPT.contains("extraction.  Add"));
        assert_eq!(LOOP_PROMPT.as_bytes(), b"It appears some entities may have still been missed. Answer Y if there are still entities that need to be added, or N if there are none. Please answer with a single letter Y or N.\n");
        assert!(CONTINUE_PROMPT.ends_with('\n'));
        assert!(LOOP_PROMPT.ends_with('\n'));
    }
}
