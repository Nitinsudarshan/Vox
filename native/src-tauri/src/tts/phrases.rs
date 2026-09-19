//! Splitting an answer into things worth speaking before the rest arrives.
//!
//! A model writes over several seconds. Waiting for it to finish makes time to
//! first audio *generation plus synthesis*; speaking each phrase as it
//! completes makes it *first phrase plus one synthesis*, and the gap widens
//! with every sentence the model goes on to write.
//!
//! So the question is where a phrase ends, and the answer has to be wrong in
//! only one direction. Splitting too late costs latency. Splitting too early
//! costs intelligibility — a synthesizer handed "Dr" produces "doctor" with a
//! falling intonation and then starts a new sentence at "Smith", which sounds
//! like a fault rather than a pause.
//!
//! Pure text, so every rule here is testable with no provider, no audio device
//! and no model.

/// Shortest phrase worth sending on its own.
///
/// Below this the overhead of a synthesis call is most of the phrase, and the
/// prosody is worse than saying it as part of the next one.
const MIN_PHRASE_CHARS: usize = 24;

/// Longest a phrase may run before it is split at a comma instead.
///
/// A speaker reading a 400-character sentence takes about 25 seconds, and
/// nothing should wait that long for its first sound.
const MAX_PHRASE_CHARS: usize = 220;

/// Abbreviations whose full stop does not end a sentence.
///
/// Deliberately short. Every entry is a judgement about text nobody has seen,
/// and a list that tries to be exhaustive gets the common cases wrong while
/// growing forever. These are the ones that appear in meeting answers.
const NON_TERMINAL: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "e.g", "i.e", "etc", "vs", "approx", "no",
];

/// Splits finished text into speakable phrases.
///
/// For text that is still being written, use [`PhraseBuffer`].
pub fn split_into_phrases(text: &str) -> Vec<String> {
    let mut buffer = PhraseBuffer::default();
    let mut phrases = buffer.push(text);
    phrases.extend(buffer.flush());
    phrases
}

/// Accumulates streamed text and yields phrases as they complete.
///
/// The streaming half of the same rules: a caller pushes whatever the model
/// just produced, takes whatever is now speakable, and calls [`Self::flush`]
/// when the model stops.
#[derive(Debug, Default)]
pub struct PhraseBuffer {
    pending: String,
}

impl PhraseBuffer {
    /// Adds text and returns whatever became speakable.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.pending.push_str(text);
        let mut phrases = Vec::new();
        while let Some(at) = self.next_boundary() {
            let phrase: String = self.pending.drain(..at).collect();
            let phrase = phrase.trim().to_string();
            if !phrase.is_empty() {
                phrases.push(phrase);
            }
        }
        phrases
    }

    /// Whatever is left, spoken or not.
    ///
    /// Called when the model stops. A trailing fragment below the minimum
    /// length is still returned here — the alternative is silently not saying
    /// the end of the answer.
    pub fn flush(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.pending).trim().to_string();
        if rest.is_empty() {
            Vec::new()
        } else {
            vec![rest]
        }
    }

    /// Whether anything is waiting to be spoken.
    pub fn is_empty(&self) -> bool {
        self.pending.trim().is_empty()
    }

    /// Byte offset just past the end of the first complete phrase.
    fn next_boundary(&self) -> Option<usize> {
        let bytes = self.pending.as_bytes();
        let mut candidate: Option<usize> = None;

        for (index, ch) in self.pending.char_indices() {
            let end = index + ch.len_utf8();
            if !matches!(ch, '.' | '!' | '?' | '\n') {
                // A comma is a fallback boundary, taken only when a sentence
                // has run long enough that waiting for its full stop costs
                // more than the flatter prosody does.
                if ch == ',' && end >= MAX_PHRASE_CHARS {
                    candidate = Some(end);
                }
                continue;
            }
            if end < MIN_PHRASE_CHARS {
                continue;
            }
            // A full stop inside a number — "1.5", "v0.1.0" — is not a
            // sentence end, and neither is one in a short abbreviation.
            if ch == '.' && (is_inside_number(bytes, index) || ends_with_abbreviation(&self.pending[..index])) {
                continue;
            }
            // A full stop with no space after it is usually a filename or a
            // domain, not a sentence. The end of the buffer is the exception:
            // there is nothing after it yet.
            if ch != '\n' && end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                continue;
            }
            return Some(end);
        }

        // Nothing terminal, but the sentence has gone on too long to keep
        // waiting: take the last comma if there was one.
        candidate.filter(|_| self.pending.len() >= MAX_PHRASE_CHARS)
    }
}

/// Whether the character before and after a full stop are both digits.
fn is_inside_number(bytes: &[u8], dot: usize) -> bool {
    let before = dot
        .checked_sub(1)
        .map(|at| bytes[at].is_ascii_digit())
        .unwrap_or(false);
    let after = bytes
        .get(dot + 1)
        .map(|byte| byte.is_ascii_digit())
        .unwrap_or(false);
    before && after
}

/// Whether the text ends with one of the abbreviations whose full stop is not
/// a sentence end.
fn ends_with_abbreviation(text: &str) -> bool {
    let tail: String = text
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let tail = tail.to_lowercase();
    NON_TERMINAL.contains(&tail.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_answer_splits_at_its_sentences() {
        let phrases = split_into_phrases(
            "We agreed to ship on Thursday. Payal is sending the deck first. \
             Anything else can wait.",
        );
        assert_eq!(phrases.len(), 3);
        assert_eq!(phrases[0], "We agreed to ship on Thursday.");
        assert_eq!(phrases[2], "Anything else can wait.");
    }

    #[test]
    fn a_phrase_is_speakable_before_the_rest_of_the_answer_exists() {
        // The whole reason the module exists: the first sentence goes to the
        // synthesizer while the model is still writing the second.
        let mut buffer = PhraseBuffer::default();
        assert!(buffer.push("We agreed to ship on ").is_empty());
        let ready = buffer.push("Thursday. Payal is");
        assert_eq!(ready, vec!["We agreed to ship on Thursday."]);
        assert!(!buffer.is_empty(), "the unfinished sentence is still waiting");
    }

    #[test]
    fn a_decimal_point_is_not_the_end_of_a_sentence() {
        let phrases = split_into_phrases("The release is version 1.5 and ships on Thursday.");
        assert_eq!(phrases.len(), 1);
    }

    #[test]
    fn an_abbreviation_does_not_start_a_new_sentence_at_the_next_word() {
        // A synthesizer handed "Dr" alone says "doctor" with a falling
        // intonation and then starts afresh at "Smith", which sounds broken.
        let phrases = split_into_phrases("Ask Dr. Smith whether the schedule still works.");
        assert_eq!(phrases.len(), 1, "got {phrases:?}");
    }

    #[test]
    fn a_full_stop_inside_a_filename_is_not_a_boundary() {
        let phrases = split_into_phrases("The notes are in meeting.notes.md and worth reading.");
        assert_eq!(phrases.len(), 1, "got {phrases:?}");
    }

    #[test]
    fn a_very_short_sentence_is_spoken_with_the_next_one() {
        // "Yes." on its own is more synthesis overhead than speech, and it
        // sounds clipped.
        let phrases = split_into_phrases("Yes. We agreed to ship the release on Thursday.");
        assert_eq!(phrases.len(), 1, "got {phrases:?}");
        assert!(phrases[0].starts_with("Yes."));
    }

    #[test]
    fn a_sentence_that_never_ends_is_split_at_a_comma_instead() {
        // Nothing should wait 25 seconds for its first sound.
        let long = format!(
            "{} , and then it kept going for quite a while longer without ever stopping",
            "we talked about the release and the schedule and the deck ".repeat(4)
        );
        let phrases = split_into_phrases(&long);
        assert!(phrases.len() >= 2, "got {} phrases", phrases.len());
        assert!(phrases[0].len() <= 400);
    }

    #[test]
    fn a_newline_ends_a_phrase_even_with_no_punctuation() {
        // Models write lists, and a bullet is a phrase whatever it ends with.
        let phrases = split_into_phrases(
            "Here is what we decided\nShip the release on Thursday\nSend the deck first",
        );
        assert_eq!(phrases.len(), 3, "got {phrases:?}");
    }

    #[test]
    fn the_end_of_an_answer_is_spoken_even_when_it_is_a_fragment() {
        // The alternative is silently not saying the last thing.
        let mut buffer = PhraseBuffer::default();
        buffer.push("We agreed to ship on Thursday. And then");
        let rest = buffer.flush();
        assert_eq!(rest, vec!["And then"]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn nothing_produces_nothing_rather_than_an_empty_utterance() {
        assert!(split_into_phrases("").is_empty());
        assert!(split_into_phrases("   \n  ").is_empty());
        let mut buffer = PhraseBuffer::default();
        assert!(buffer.push("  ").is_empty());
        assert!(buffer.flush().is_empty());
    }

    #[test]
    fn splitting_loses_none_of_the_words() {
        // The property that matters most: whatever the rules do, the answer
        // spoken must be the answer written.
        let text = "We agreed to ship on Thursday. Payal is sending the deck. \
                    Version 1.5 goes out at noon, and Dr. Rao will review it.";
        let rejoined = split_into_phrases(text).join(" ");
        let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(normalize(&rejoined), normalize(text));
    }

    #[test]
    fn streaming_in_awkward_chunks_gives_the_same_phrases_as_one_block() {
        // A model streams tokens, not sentences, and a boundary landing
        // mid-word must not change the result.
        let text = "We agreed to ship on Thursday. Payal is sending the deck first.";
        let whole = split_into_phrases(text);

        let mut buffer = PhraseBuffer::default();
        let mut streamed = Vec::new();
        for chunk in text.as_bytes().chunks(3) {
            streamed.extend(buffer.push(std::str::from_utf8(chunk).unwrap()));
        }
        streamed.extend(buffer.flush());
        assert_eq!(streamed, whole);
    }
}
