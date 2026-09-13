//! Streaming line-break opportunities, implemented from Unicode 15.0 UAX #14.
//!
//! The rule numbers below refer to revision 49 of the Unicode specification:
//! <https://www.unicode.org/reports/tr14/tr14-49.html>. These are the default
//! rules, including LB25's pair rules; the optional numeric-expression tailoring
//! in section 8.2 is not applied. Character properties come from the Unicode
//! data files, while the algorithm and its bounded state are maintained here.

use super::unicode_break_data::{properties, LineBreakClass as Class};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BreakOpportunity {
    Allowed,
    Mandatory,
}

/// An effective character after LB1 and the combining-sequence rules LB9–LB10.
#[derive(Clone, Copy, Debug)]
struct Character {
    class: Class,
    east_asian_fwh: bool,
    unassigned_extended_pictographic: bool,
}

/// Exact streaming context for the next scalar in one logical text stream.
///
/// The state has constant size, even for arbitrary runs of spaces, combining
/// marks or regional indicators. Cloning it creates an exact resume checkpoint.
/// A fresh state means start of text, not an arbitrary slice of an existing line.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LineBreakState {
    previous: Option<Character>,
    previous_raw: Option<Class>,
    before_previous: Option<Class>,
    previous_non_space: Option<Class>,
    regional_odd: bool,
}

impl LineBreakState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Feed a scalar, returning the opportunity immediately before that scalar.
    /// Calling this across UTF-8 chunks is identical to feeding their concatenation.
    pub(super) fn push(&mut self, scalar: char) -> Option<BreakOpportunity> {
        let property = properties(scalar);
        // LB1: resolve the classes whose default interpretation is contextual.
        let raw = match property.class {
            Class::AI | Class::SG | Class::XX => Class::AL,
            Class::SA if property.mark => Class::CM,
            Class::SA => Class::AL,
            Class::CJ => Class::NS,
            class => class,
        };
        let combining = matches!(raw, Class::CM | Class::ZWJ);
        let inherits = combining
            && self.previous.is_some_and(|previous| {
                !matches!(
                    previous.class,
                    Class::BK | Class::CR | Class::LF | Class::NL | Class::SP | Class::ZW
                )
            });
        let current = Character {
            // LB10 applies when the preceding character cannot be a base.
            class: if combining { Class::AL } else { raw },
            east_asian_fwh: property.east_asian_fwh,
            unassigned_extended_pictographic: property.unassigned_extended_pictographic,
        };
        let opportunity = self.before(current, raw, inherits);
        self.previous_raw = Some(raw);
        // LB9 ignores attached CM/ZWJ in every subsequent rule's context. In
        // particular, they neither reset Hebrew-hyphen context nor count as RI.
        if !inherits {
            self.before_previous = self.previous.map(|previous| previous.class);
            self.previous = Some(current);
            if current.class != Class::SP {
                self.previous_non_space = Some(current.class);
            }
            self.regional_odd = current.class == Class::RI && !self.regional_odd;
        }
        opportunity
    }

    /// LB3: end of text always supplies a mandatory opportunity, including for
    /// an empty stream. This does not consume or mutate the checkpoint.
    #[cfg(test)]
    pub(super) fn finish(&self) -> BreakOpportunity {
        BreakOpportunity::Mandatory
    }

    fn before(&self, current: Character, raw: Class, inherits: bool) -> Option<BreakOpportunity> {
        use BreakOpportunity::{Allowed, Mandatory};
        use Class::*;
        let previous = self.previous?; // LB2: no break at start of text.
        let left = previous.class;
        let right = current.class;

        // LB28 covers the common alphabetic pair immediately. The only earlier
        // rules that can apply here are LB8a/LB9 for a joiner or attached mark,
        // and those also forbid a break. Hard breaks, SP and ZW have distinct
        // effective classes, so none of their earlier break rules is bypassed.
        if alphabetic(left) && alphabetic(right) {
            return None;
        }

        // LB4–LB7: mandatory breaks take precedence over ordinary opportunities.
        if self.previous_raw == Some(BK) {
            return Some(Mandatory);
        }
        if self.previous_raw == Some(CR) && raw == LF {
            return None;
        }
        if matches!(self.previous_raw, Some(CR | LF | NL)) {
            return Some(Mandatory);
        }
        if matches!(raw, BK | CR | LF | NL | SP | ZW) {
            return None;
        }
        // LB8 remembers only the final non-space character, not the space run.
        if self.previous_non_space == Some(ZW) {
            return Some(Allowed);
        }
        // LB8a examines the actual preceding scalar, before LB9 removes joiners.
        if self.previous_raw == Some(ZWJ) || inherits {
            return None;
        }
        // LB11–LB13.
        if left == WJ || right == WJ || left == GL {
            return None;
        }
        if right == GL && !matches!(left, SP | BA | HY) {
            return None;
        }
        if matches!(right, CL | CP | EX | IS | SY) {
            return None;
        }
        // LB14–LB17 also prohibit breaks across any number of spaces.
        if self.previous_non_space == Some(OP)
            || (self.previous_non_space == Some(QU) && right == OP)
            || (matches!(self.previous_non_space, Some(CL | CP)) && right == NS)
            || (self.previous_non_space == Some(B2) && right == B2)
        {
            return None;
        }
        // LB18–LB20.
        if left == SP {
            return Some(Allowed);
        }
        if left == QU || right == QU {
            return None;
        }
        if left == CB || right == CB {
            return Some(Allowed);
        }
        // LB21, LB21a, LB21b, LB22.
        if matches!(right, BA | HY | NS | IN)
            || left == BB
            || (self.before_previous == Some(HL) && matches!(left, HY | BA))
            || (left == SY && right == HL)
        {
            return None;
        }
        // LB23–LB24: letters, digits, ideographs and numeric affixes.
        if (alphabetic(left) && right == NU)
            || (left == NU && alphabetic(right))
            || (left == PR && matches!(right, ID | EB | EM))
            || (matches!(left, ID | EB | EM) && right == PO)
            || (matches!(left, PR | PO) && alphabetic(right))
            || (alphabetic(left) && matches!(right, PR | PO))
        {
            return None;
        }
        // LB25: the default rule's explicit numeric pairs.
        if (matches!(left, CL | CP | NU) && matches!(right, PO | PR))
            || (matches!(left, PO | PR) && matches!(right, OP | NU))
            || (matches!(left, HY | IS | NU | SY) && right == NU)
        {
            return None;
        }
        // LB26–LB27: preserve Korean syllable blocks and numeric affixes.
        if (left == JL && matches!(right, JL | JV | H2 | H3))
            || (matches!(left, JV | H2) && matches!(right, JV | JT))
            || (matches!(left, JT | H3) && right == JT)
            || (hangul(left) && right == PO)
            || (left == PR && hangul(right))
        {
            return None;
        }
        // LB29–LB30: alphabetics and non-East-Asian parentheses.
        if (left == IS && alphabetic(right))
            || ((alphabetic(left) || left == NU) && right == OP && !current.east_asian_fwh)
            || (left == CP && !previous.east_asian_fwh && (alphabetic(right) || right == NU))
        {
            return None;
        }
        // LB30a counts RI modulo two; combining characters do not affect it.
        if left == RI && right == RI && self.regional_odd {
            return None;
        }
        // LB30b includes reserved code points intended for future emoji.
        if right == EM && (left == EB || previous.unassigned_extended_pictographic) {
            return None;
        }
        Some(Allowed) // LB31.
    }
}

fn alphabetic(class: Class) -> bool {
    matches!(class, Class::AL | Class::HL)
}

fn hangul(class: Class) -> bool {
    matches!(
        class,
        Class::JL | Class::JV | Class::JT | Class::H2 | Class::H3
    )
}

#[cfg(test)]
pub(super) fn linebreaks(text: &str) -> LineBreaks<'_> {
    LineBreaks {
        characters: text.char_indices(),
        state: LineBreakState::new(),
        end: Some(text.len()),
    }
}

#[cfg(test)]
pub(super) struct LineBreaks<'a> {
    characters: std::str::CharIndices<'a>,
    state: LineBreakState,
    end: Option<usize>,
}

#[cfg(test)]
impl Iterator for LineBreaks<'_> {
    type Item = (usize, BreakOpportunity);

    fn next(&mut self) -> Option<Self::Item> {
        for (offset, scalar) in self.characters.by_ref() {
            if let Some(opportunity) = self.state.push(scalar) {
                return Some((offset, opportunity));
            }
        }
        self.end.take().map(|end| (end, self.state.finish()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE_BREAK_TESTS: &str = include_str!("../../../data/unicode/15.0.0/LineBreakTest.txt");

    fn fixture_cases(
    ) -> impl Iterator<Item = (usize, String, Vec<(usize, bool)>, Vec<&'static str>)> {
        LINE_BREAK_TESTS
            .lines()
            .enumerate()
            .filter_map(|(line_number, line)| {
                let (sequence, comment) = line.split_once('#')?;
                if sequence.trim().is_empty() {
                    return None;
                }
                let mut text = String::new();
                let mut boundaries = Vec::new();
                for token in sequence.split_whitespace() {
                    if matches!(token, "×" | "÷") {
                        boundaries.push((text.len(), token == "÷"));
                    } else {
                        text.push(char::from_u32(u32::from_str_radix(token, 16).unwrap()).unwrap());
                    }
                }
                let rules = comment
                    .split('[')
                    .skip(1)
                    .map(|part| part.split_once(']').unwrap().0)
                    .collect();
                Some((line_number + 1, text, boundaries, rules))
            })
    }

    #[test]
    fn official_unicode_15_fixture_with_documented_numeric_policy_differences() {
        // The official fixture explicitly uses §8.2 Example 7's optional
        // numeric-expression tailoring. Viem uses the default LB25 pairs. No
        // test case is skipped: only individual numeric-rule differences may
        // be adjusted, with the fixture's rule annotation checked as well.
        let mut cases = 0;
        let mut default_pair_differences = 0;
        let mut tailored_expression_differences = 0;
        for (line_number, text, expected, rules) in fixture_cases() {
            cases += 1;
            let mut state = LineBreakState::new();
            for ((offset, scalar), ((expected_offset, expected_break), rule)) in
                text.char_indices().zip(expected.iter().zip(&rules))
            {
                assert_eq!(offset, *expected_offset);
                let previous_class = state.previous.map(|previous| previous.class);
                let actual_break = state.push(scalar).is_some();
                if actual_break == *expected_break {
                    continue;
                }
                let current_class = state.previous.unwrap().class;
                use Class::*;
                let default_numeric_pair = previous_class.is_some_and(|left| {
                    (matches!(left, CL | CP | NU) && matches!(current_class, PO | PR))
                        || (matches!(left, PO | PR) && matches!(current_class, OP | NU))
                        || (matches!(left, HY | IS | NU | SY) && current_class == NU)
                });
                if !actual_break && *expected_break && default_numeric_pair && *rule == "999.0" {
                    default_pair_differences += 1;
                } else if actual_break && !expected_break && rule.starts_with("25.") {
                    tailored_expression_differences += 1;
                } else {
                    panic!("Unicode fixture line {line_number}, byte {offset}, rule {rule}, text {text:?}: expected break {expected_break}, actual {actual_break}, previous {previous_class:?}, current {current_class:?}");
                }
            }
            assert_eq!(expected.last(), Some(&(text.len(), true)));
            assert_eq!(state.finish(), BreakOpportunity::Mandatory);
        }
        // Pin the policy delta so an unrelated regression cannot silently grow
        // the set of accepted fixture differences.
        assert_eq!(cases, 7_654);
        assert_eq!(default_pair_differences, 34);
        assert_eq!(tailored_expression_differences, 0);
    }

    fn assert_restart_at_breaks(text: &str) {
        let all = linebreaks(text).collect::<Vec<_>>();
        for &(split, _) in &all {
            let expected = all
                .iter()
                .copied()
                .filter(|&(offset, _)| offset > split)
                .collect::<Vec<_>>();
            let resumed = if split == text.len() {
                Vec::new()
            } else {
                linebreaks(&text[split..])
                    .map(|(offset, kind)| (split + offset, kind))
                    .collect()
            };
            assert_eq!(resumed, expected, "restart at {split} in {text:?}");
        }
    }

    #[test]
    fn restarting_at_every_emitted_break_preserves_fixture_suffixes() {
        for (_, text, _, _) in fixture_cases() {
            assert_restart_at_breaks(&text);
        }
    }

    #[test]
    fn restarting_at_breaks_preserves_all_representative_class_triples() {
        let mut representatives = Vec::new();
        for (_, text, _, _) in fixture_cases() {
            for scalar in text.chars() {
                let class = properties(scalar).class;
                if !representatives.iter().any(|&(seen, _)| seen == class) {
                    representatives.push((class, scalar));
                }
            }
        }
        // Unicode 15 has 43 classes; SG contains only surrogate code points,
        // which cannot occur in Rust's scalar-valued UTF-8 text.
        assert_eq!(representatives.len(), 42);
        for &(_, a) in &representatives {
            for &(_, b) in &representatives {
                for &(_, c) in &representatives {
                    assert_restart_at_breaks(&[a, b, c].into_iter().collect::<String>());
                }
            }
        }
    }

    #[test]
    fn mandatory_breaks_and_end_of_text() {
        use BreakOpportunity::*;
        assert_eq!(linebreaks("").collect::<Vec<_>>(), [(0, Mandatory)]);
        assert_eq!(
            linebreaks("a\r\nb\rc\nd").collect::<Vec<_>>(),
            [
                (3, Mandatory),
                (5, Mandatory),
                (7, Mandatory),
                (8, Mandatory),
            ]
        );
    }

    #[test]
    fn preserves_bases_through_combining_marks_and_joiners() {
        use BreakOpportunity::*;
        for text in [
            "(\u{301}   a",
            "א\u{301}-\u{301}א",
            "\u{1f1e6}\u{301}\u{1f1e7}",
            "🧑\u{301}🏽",
        ] {
            assert_eq!(
                linebreaks(text).collect::<Vec<_>>(),
                [(text.len(), Mandatory)],
                "{text:?}"
            );
        }
        assert_eq!(
            linebreaks("a\u{200b}   b").collect::<Vec<_>>(),
            [(7, Allowed), (8, Mandatory)]
        );
    }

    #[test]
    fn east_asian_parentheses_and_regional_pairs() {
        use BreakOpportunity::*;
        assert_eq!(
            linebreaks("a（b").collect::<Vec<_>>(),
            [(1, Allowed), (5, Mandatory)]
        );
        assert_eq!(
            linebreaks("🇦🇧🇨🇩🇪").collect::<Vec<_>>(),
            [(8, Allowed), (16, Allowed), (20, Mandatory)]
        );
    }

    #[test]
    fn copied_checkpoint_resumes_exactly_at_every_scalar_boundary() {
        let text = "a \u{200b}   (\u{301}a) א-א 🇦\u{301}🇧🇨 🧑🏽\r\nnext";
        for (split, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), '\0')))
        {
            let mut state = LineBreakState::new();
            let mut actual = Vec::new();
            for (offset, scalar) in text[..split].char_indices() {
                if let Some(opportunity) = state.push(scalar) {
                    actual.push((offset, opportunity));
                }
            }
            let mut resumed = state;
            for (offset, scalar) in text[split..].char_indices() {
                if let Some(opportunity) = resumed.push(scalar) {
                    actual.push((split + offset, opportunity));
                }
            }
            actual.push((text.len(), resumed.finish()));
            assert_eq!(
                actual,
                linebreaks(text).collect::<Vec<_>>(),
                "split {split}"
            );
        }
        assert!(std::mem::size_of::<LineBreakState>() <= 32);
    }

    #[test]
    fn default_numeric_pairs_are_kept_without_optional_expression_tailoring() {
        use BreakOpportunity::*;
        for text in ["$[", ")%", ")$", "/9", ",9"] {
            assert_eq!(
                linebreaks(text).collect::<Vec<_>>(),
                [(text.len(), Mandatory)],
                "{text:?}"
            );
        }
        // Conversely, the optional tailoring would keep this entire numeric
        // expression together; the default pair rules allow a break before %.
        assert_eq!(
            linebreaks("1,%").collect::<Vec<_>>(),
            [(2, Allowed), (3, Mandatory)]
        );
    }

    #[test]
    fn default_class_resolution_preserves_alphabetic_and_mark_semantics() {
        use BreakOpportunity::*;
        for text in ["a§b", "a\u{e000}b", "aกb", "(ั   a", "aぁ"] {
            assert_eq!(
                linebreaks(text).collect::<Vec<_>>(),
                [(text.len(), Mandatory)],
                "{text:?}"
            );
        }
    }
}
