#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversionWarning {
    UnrepresentableCharacters {
        encoding: super::Encoding,
        count: usize,
    },
    RepairedTruncatedUtf16,
}
impl ConversionWarning {
    pub fn message(&self) -> String {
        match self {
            Self::UnrepresentableCharacters { count, .. } => format!("Warning: {count} character{} could not be represented in Latin-1 and {} replaced with '?'.", if *count == 1 { "" } else { "s" }, if *count == 1 { "was" } else { "were" }),
            Self::RepairedTruncatedUtf16 => "Repaired an incomplete UTF-16 character to continue typing. Undo restores the original bytes.".to_owned(),
        }
    }
}
