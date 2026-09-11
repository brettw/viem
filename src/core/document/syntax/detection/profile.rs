//! Finite declarative content rules. Compilation is independent of document
//! contents; all rules share one pass over the already bounded sampled lines.
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock};

pub const CONTENT_RULE_LIMIT: usize = 64;
pub const CONTENT_LITERAL_BYTE_LIMIT: usize = 4096;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ContentMatch {
    /// A literal anywhere within one inspected complete line.
    Contains,
    /// A literal after optional whitespace on logical line zero.
    FirstLinePrefix,
    /// A literal after leading whitespace in the first nonempty sampled line.
    SamplePrefix,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ContentSignature {
    pub language: String,
    pub literal: String,
    pub matching: ContentMatch,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExtensionDisambiguator {
    /// Case-sensitive extension without the leading dot.
    pub extension: String,
    pub signature: ContentSignature,
}
#[derive(Clone, Debug, Default)]
struct State {
    edges: BTreeMap<u8, usize>,
    failure: usize,
    outputs: u64,
}
#[derive(Clone, Debug)]
pub struct DetectionProfile {
    signatures: Vec<ContentSignature>,
    disambiguators: Vec<ExtensionDisambiguator>,
    states: Vec<State>,
}
impl Default for DetectionProfile {
    fn default() -> Self {
        (*bundled_profile()).clone()
    }
}

impl DetectionProfile {
    /// Compile an exact ordered policy. Earlier rules have precedence; callers
    /// can start with Default and register extra rules ahead of the builtins.
    pub fn compile(
        signatures: Vec<ContentSignature>,
        disambiguators: Vec<ExtensionDisambiguator>,
    ) -> Result<Self, String> {
        let valid_language = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+.#-".contains(&b))
        };
        let mut literal_bytes = 0;
        if signatures.len() + disambiguators.len() > CONTENT_RULE_LIMIT {
            return Err("Content detection rule count exceeds 64".into());
        }
        for signature in disambiguators
            .iter()
            .map(|d| &d.signature)
            .chain(&signatures)
        {
            literal_bytes += signature.literal.len();
            if !valid_language(&signature.language)
                || signature.literal.is_empty()
                || signature.literal.len() > 256
                || signature.literal.contains(['\n', '\r', '\0'])
            {
                return Err("Invalid content detection language or literal".into());
            }
        }
        if literal_bytes > CONTENT_LITERAL_BYTE_LIMIT {
            return Err("Content detection literals exceed 4096 bytes".into());
        }
        if disambiguators.iter().any(|d| {
            d.extension.is_empty()
                || d.extension.len() > 128
                || !d
                    .extension
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
        }) {
            return Err("Invalid content detection extension".into());
        }
        let mut this = Self {
            signatures,
            disambiguators,
            states: vec![State::default()],
        };
        let literals = this.rules().map(|s| s.literal.clone()).collect::<Vec<_>>();
        for (index, literal) in literals.iter().enumerate() {
            let mut state = 0;
            for byte in literal.bytes() {
                let next = if let Some(next) = this.states[state].edges.get(&byte) {
                    *next
                } else {
                    let next = this.states.len();
                    this.states.push(State::default());
                    this.states[state].edges.insert(byte, next);
                    next
                };
                state = next;
            }
            this.states[state].outputs |= 1u64 << index;
        }
        let mut queue = this.states[0]
            .edges
            .values()
            .copied()
            .collect::<VecDeque<_>>();
        while let Some(parent) = queue.pop_front() {
            let edges = this.states[parent].edges.clone();
            for (byte, child) in edges {
                let mut failure = this.states[parent].failure;
                while failure != 0 && !this.states[failure].edges.contains_key(&byte) {
                    failure = this.states[failure].failure;
                }
                let failure = this.states[failure].edges.get(&byte).copied().unwrap_or(0);
                this.states[child].failure = failure;
                this.states[child].outputs |= this.states[failure].outputs;
                queue.push_back(child);
            }
        }
        Ok(this)
    }
    /// Registration is bounded and atomic; the most recently registered rule
    /// precedes existing rules. It cannot execute a callback or regular expression.
    pub fn register_signature(&mut self, signature: ContentSignature) -> Result<(), String> {
        let mut signatures = self.signatures.clone();
        signatures.insert(0, signature);
        *self = Self::compile(signatures, self.disambiguators.clone())?;
        Ok(())
    }
    pub fn register_disambiguator(&mut self, rule: ExtensionDisambiguator) -> Result<(), String> {
        let mut rules = self.disambiguators.clone();
        rules.insert(0, rule);
        *self = Self::compile(self.signatures.clone(), rules)?;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn states_len_for_test(&self) -> usize {
        self.states.len()
    }
    fn rules(&self) -> impl Iterator<Item = &ContentSignature> {
        self.disambiguators
            .iter()
            .map(|d| &d.signature)
            .chain(&self.signatures)
    }
    pub(super) fn scan(&self, samples: &[(usize, String)]) -> (u64, usize) {
        let rules = self.rules().collect::<Vec<_>>();
        let first_content = samples
            .iter()
            .position(|(_, line)| !line.trim_start().is_empty());
        let mut found = 0u64;
        let mut steps = 0;
        for (sample, (line_number, line)) in samples.iter().enumerate() {
            let leading = line.len() - line.trim_start().len();
            let mut eligible = 0u64;
            for (index, rule) in rules.iter().enumerate() {
                let allowed = match rule.matching {
                    ContentMatch::Contains => true,
                    ContentMatch::FirstLinePrefix => *line_number == 0,
                    ContentMatch::SamplePrefix => Some(sample) == first_content,
                };
                if allowed {
                    eligible |= 1u64 << index;
                }
            }
            let mut state = 0;
            for (at, byte) in line.bytes().enumerate() {
                steps += 1;
                while state != 0 && !self.states[state].edges.contains_key(&byte) {
                    state = self.states[state].failure;
                    steps += 1;
                }
                state = self.states[state].edges.get(&byte).copied().unwrap_or(0);
                let mut outputs = self.states[state].outputs & eligible & !found;
                while outputs != 0 {
                    let index = outputs.trailing_zeros() as usize;
                    outputs &= outputs - 1;
                    eligible &= !(1u64 << index);
                    steps += 1;
                    let rule = rules[index];
                    let start = at + 1 - rule.literal.len();
                    let matches = match rule.matching {
                        ContentMatch::Contains => true,
                        ContentMatch::FirstLinePrefix => *line_number == 0 && start == leading,
                        ContentMatch::SamplePrefix => {
                            Some(sample) == first_content && start == leading
                        }
                    };
                    if matches {
                        found |= 1u64 << index;
                    }
                }
            }
        }
        (found, steps)
    }
    pub(super) fn extension<'a>(&'a self, extension: &str, found: u64) -> Option<&'a str> {
        self.disambiguators
            .iter()
            .enumerate()
            .find(|(index, rule)| rule.extension == extension && found & (1u64 << index) != 0)
            .map(|(_, rule)| rule.signature.language.as_str())
    }
    pub(super) fn signature(&self, found: u64) -> Option<&str> {
        self.signatures
            .iter()
            .enumerate()
            .find(|(index, _)| found & (1u64 << (self.disambiguators.len() + index)) != 0)
            .map(|(_, rule)| rule.language.as_str())
    }
}

pub fn bundled_profile() -> Arc<DetectionProfile> {
    static PROFILE: OnceLock<Arc<DetectionProfile>> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            let signature = |language: &str, literal: &str, matching| ContentSignature {
                language: language.into(),
                literal: literal.into(),
                matching,
            };
            let signatures = vec![
                signature("php", "<?php", ContentMatch::FirstLinePrefix),
                signature("xml", "<?xml", ContentMatch::FirstLinePrefix),
            ];
            let mut rules = Vec::new();
            for (extension, language, literal, matching) in [
                ("h", "objc", "@interface", ContentMatch::Contains),
                ("h", "cpp", "namespace ", ContentMatch::Contains),
                ("h", "cpp", "class ", ContentMatch::Contains),
                ("h", "cpp", "template<", ContentMatch::Contains),
                ("h", "cpp", "template <", ContentMatch::Contains),
                ("m", "matlab", "%", ContentMatch::SamplePrefix),
                ("m", "matlab", "function ", ContentMatch::SamplePrefix),
            ] {
                rules.push(ExtensionDisambiguator {
                    extension: extension.into(),
                    signature: signature(language, literal, matching),
                });
            }
            Arc::new(
                DetectionProfile::compile(signatures, rules)
                    .expect("bounded built-in detection policy"),
            )
        })
        .clone()
}
