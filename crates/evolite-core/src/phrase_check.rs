//! "Check the phrase": questions about a few words of the phrase and the check of the
//! answers, both in the core. The interface gets numbers and choices, never the phrase; the
//! right words stay here until the answers come.

use evolite_api::PhraseQuestion;
use zeroize::Zeroizing;

/// How many words are asked, and how many choices each has.
pub const ASKED: usize = 4;
pub const CHOICES: usize = 3;

/// The right words of a check, by question, wiped when dropped.
pub struct Answers(Vec<Zeroizing<String>>);

/// `ASKED` positions of `words` (in order), each with the right word and `CHOICES - 1` other
/// words of the BIP39 list — not of the phrase, so the screen shows no more of it than the
/// asked words — in a random place. `dice(n)` is a random number below `n`.
pub fn questions(
    words: &[&str],
    dice: &mut impl FnMut(usize) -> usize,
) -> (Vec<PhraseQuestion>, Answers) {
    let list: Vec<&str> = crate::bip39::words().collect();
    let mut asked: Vec<usize> = Vec::new();
    while asked.len() < ASKED.min(words.len()) {
        let n = dice(words.len());
        if !asked.contains(&n) {
            asked.push(n);
        }
    }
    asked.sort_unstable();
    let mut answers = Vec::new();
    let questions = asked
        .into_iter()
        .map(|i| {
            let right = words[i];
            let mut choices: Vec<String> = Vec::new();
            while choices.len() < CHOICES - 1 {
                let other = list[dice(list.len())];
                if other != right && !choices.iter().any(|c| c == other) {
                    choices.push(other.to_owned());
                }
            }
            choices.insert(dice(CHOICES), right.to_owned());
            answers.push(Zeroizing::new(right.to_owned()));
            PhraseQuestion {
                number: i + 1,
                choices,
            }
        })
        .collect();
    (questions, Answers(answers))
}

/// The number (from 1) of the first question answered wrong, if any; a missing answer is
/// wrong. Says nothing about what the right word was.
pub fn first_wrong(
    questions: &[PhraseQuestion],
    answers: &Answers,
    picks: &[String],
) -> Option<usize> {
    questions
        .iter()
        .zip(&answers.0)
        .enumerate()
        .find(|(i, (_, right))| picks.get(*i).is_none_or(|p| p != right.as_str()))
        .map(|(_, (q, _))| q.number)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: [&str; 12] = [
        "orbit", "canvas", "lunar", "pepper", "alert", "frozen", "museum", "tackle", "ribbon",
        "velvet", "island", "sugar",
    ];

    /// A plain generator, so the tests repeat.
    fn dice(seed: u64) -> impl FnMut(usize) -> usize {
        let mut s = seed | 1;
        move |n| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % n as u64) as usize
        }
    }

    #[test]
    fn four_words_three_choices_one_right() {
        for seed in 0..50 {
            let (qs, answers) = questions(&PHRASE, &mut dice(seed));
            assert_eq!(qs.len(), ASKED);
            assert!(qs.windows(2).all(|w| w[0].number < w[1].number));
            for (q, right) in qs.iter().zip(&answers.0) {
                assert_eq!(right.as_str(), PHRASE[q.number - 1]);
                assert_eq!(q.choices.len(), CHOICES);
                assert_eq!(q.choices.iter().filter(|c| *c == right.as_str()).count(), 1);
                // The other choices are list words, but none is another word of the phrase
                // unless by chance it is the same as one: never the asked one.
                assert!(q.choices.iter().all(|c| crate::bip39::index(c).is_some()));
            }
        }
    }

    #[test]
    fn the_right_picks_pass_and_a_wrong_one_is_named() {
        let (qs, answers) = questions(&PHRASE, &mut dice(7));
        let right: Vec<String> = qs.iter().map(|q| PHRASE[q.number - 1].to_owned()).collect();
        assert_eq!(first_wrong(&qs, &answers, &right), None);
        let mut wrong = right.clone();
        wrong[2] = qs[2]
            .choices
            .iter()
            .find(|c| **c != right[2])
            .unwrap()
            .clone();
        assert_eq!(first_wrong(&qs, &answers, &wrong), Some(qs[2].number));
        assert_eq!(first_wrong(&qs, &answers, &right[..3]), Some(qs[3].number));
    }
}
