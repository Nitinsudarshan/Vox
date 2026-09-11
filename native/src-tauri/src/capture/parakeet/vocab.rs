//! Parakeet's token vocabulary, and turning token ids back into text.
//!
//! `vocab.txt` ships beside the ONNX graphs. Each line is a SentencePiece
//! token and its id, separated by a space:
//!
//! ```text
//! <unk> 0
//! ▁the 1
//! s 2
//! ▁ 3
//! <blk> 1024
//! ```
//!
//! The `▁` (U+2581, "lower one eighth block") is SentencePiece's word-start
//! marker, not an underscore. Every piece of this file exists because that
//! marker has to become an ordinary space at exactly the right moments —
//! joining the tokens naively produces `▁the▁cat▁sat` and replacing every
//! marker with a space produces ` the cat sat . ` with a space before the
//! full stop.

use std::collections::HashMap;
use std::path::Path;

/// SentencePiece's word-start marker.
const WORD_START: char = '\u{2581}';

/// The token that means "emit nothing at this frame".
const BLANK_TOKEN: &str = "<blk>";

#[derive(Debug, thiserror::Error)]
pub enum VocabError {
    #[error("could not read {path}: {message}")]
    Read { path: String, message: String },

    #[error("{path} line {line} is not '<token> <id>'")]
    Malformed { path: String, line: usize },

    #[error("{path} has no '<blk>' token, so it is not a vocabulary this decoder understands")]
    NoBlank { path: String },
}

/// The token table for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct Vocab {
    /// Tokens by id, already with `▁` turned into a leading space.
    tokens: Vec<String>,
    blank: usize,
}

impl Vocab {
    /// Reads a `vocab.txt`.
    pub fn load(path: &Path) -> Result<Self, VocabError> {
        let text = std::fs::read_to_string(path).map_err(|e| VocabError::Read {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Self::parse(&text).map_err(|kind| match kind {
            ParseFailure::Malformed(line) => VocabError::Malformed {
                path: path.display().to_string(),
                line,
            },
            ParseFailure::NoBlank => VocabError::NoBlank {
                path: path.display().to_string(),
            },
        })
    }

    /// Parses vocabulary text. Split out so the format is testable without a file.
    pub fn parse(text: &str) -> Result<Self, ParseFailure> {
        let mut by_id: HashMap<usize, String> = HashMap::new();
        let mut blank = None;

        for (index, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            // `rsplit_once`, not `split_once`: the token itself can be a
            // space, and `▁ 3` split from the left yields an empty token and
            // an id of "3" only by luck. From the right it is always correct.
            let (token, id) = line
                .rsplit_once(' ')
                .ok_or(ParseFailure::Malformed(index + 1))?;
            let id: usize = id.parse().map_err(|_| ParseFailure::Malformed(index + 1))?;

            if token == BLANK_TOKEN {
                blank = Some(id);
            }
            by_id.insert(id, token.replace(WORD_START, " "));
        }

        let blank = blank.ok_or(ParseFailure::NoBlank)?;
        let size = by_id.keys().copied().max().map(|m| m + 1).unwrap_or(0);
        let mut tokens = vec![String::new(); size];
        for (id, token) in by_id {
            tokens[id] = token;
        }

        Ok(Self { tokens, blank })
    }

    /// How many token ids the model's logit vector covers.
    ///
    /// The TDT joint emits vocabulary logits followed by duration logits, and
    /// this is where the split falls.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The id that means "emit nothing".
    pub fn blank(&self) -> usize {
        self.blank
    }

    /// One token's text, or `""` for an id the vocabulary does not cover.
    pub fn token(&self, id: usize) -> &str {
        self.tokens.get(id).map(String::as_str).unwrap_or("")
    }

    /// Joins decoded ids into readable text.
    pub fn detokenize(&self, ids: &[usize]) -> String {
        let joined: String = ids.iter().map(|id| self.token(*id)).collect();
        tidy_spaces(&joined)
    }
}

/// Why parsing failed, without a path attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseFailure {
    Malformed(usize),
    NoBlank,
}

/// Removes the spaces SentencePiece's marker leaves in the wrong places.
///
/// A space survives only when a word character follows it. That is the whole
/// rule, and it covers the three cases the marker gets wrong: the leading
/// space on the first token, the space before punctuation (`the cat sat .`),
/// and a trailing space at the end of an utterance.
fn tidy_spaces(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());

    for (index, &ch) in chars.iter().enumerate() {
        if ch != ' ' {
            out.push(ch);
            continue;
        }
        let next_is_word = chars
            .get(index + 1)
            .map(|c| c.is_alphanumeric() || *c == '\'')
            .unwrap_or(false);
        // Never a leading space, and never two in a row.
        if next_is_word && !out.is_empty() {
            out.push(' ');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vocabulary shaped like the real one, small enough to reason about.
    fn vocab() -> Vocab {
        Vocab::parse(
            "<unk> 0\n\u{2581}the 1\n\u{2581}cat 2\n\u{2581}sat 3\ns 4\n. 5\n\u{2581} 6\n<blk> 7\n",
        )
        .unwrap()
    }

    #[test]
    fn ids_are_placed_at_their_stated_index_not_their_line_number() {
        let parsed = Vocab::parse("b 1\na 0\n<blk> 2\n").unwrap();
        assert_eq!(parsed.token(0), "a");
        assert_eq!(parsed.token(1), "b");
        assert_eq!(parsed.blank(), 2);
    }

    #[test]
    fn the_word_start_marker_becomes_a_space() {
        assert_eq!(vocab().token(1), " the");
    }

    #[test]
    fn a_token_that_is_only_the_marker_is_a_bare_space() {
        // Splitting from the left would read this line's token as empty.
        assert_eq!(vocab().token(6), " ");
    }

    #[test]
    fn detokenizing_drops_the_leading_space_and_keeps_the_inner_ones() {
        assert_eq!(vocab().detokenize(&[1, 2, 3]), "the cat sat");
    }

    #[test]
    fn a_space_before_punctuation_is_removed() {
        // "▁the ▁cat ▁sat ▁ ." would otherwise end "sat ."
        assert_eq!(vocab().detokenize(&[1, 2, 3, 6, 5]), "the cat sat.");
    }

    #[test]
    fn a_subword_joins_the_word_before_it() {
        assert_eq!(vocab().detokenize(&[1, 2, 4]), "the cats");
    }

    #[test]
    fn nothing_decoded_is_the_empty_string_not_a_space() {
        assert_eq!(vocab().detokenize(&[]), "");
        assert_eq!(vocab().detokenize(&[6]), "");
    }

    #[test]
    fn an_id_outside_the_vocabulary_contributes_nothing() {
        assert_eq!(vocab().token(999), "");
        assert_eq!(vocab().detokenize(&[1, 999, 2]), "the cat");
    }

    #[test]
    fn the_length_covers_every_id_including_gaps() {
        // The logit split depends on this: a vocabulary that reports 2 when
        // the model emits 8 token logits reads duration logits as tokens.
        let sparse = Vocab::parse("a 0\n<blk> 7\n").unwrap();
        assert_eq!(sparse.len(), 8);
    }

    #[test]
    fn a_vocabulary_without_a_blank_is_refused() {
        assert_eq!(Vocab::parse("a 0\nb 1\n"), Err(ParseFailure::NoBlank));
    }

    #[test]
    fn a_line_that_is_not_token_and_id_is_refused_with_its_line_number() {
        assert_eq!(Vocab::parse("a 0\nnope\n<blk> 1\n"), Err(ParseFailure::Malformed(2)));
        assert_eq!(
            Vocab::parse("a 0\nb notanumber\n<blk> 1\n"),
            Err(ParseFailure::Malformed(2))
        );
    }

    #[test]
    fn blank_lines_are_skipped_rather_than_failing_the_file() {
        assert!(Vocab::parse("a 0\n\n<blk> 1\n").is_ok());
    }
}
