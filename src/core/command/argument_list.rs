//! Portable argument-list selection. Hosts supply file identity matches; this
//! module owns count arithmetic and the view's remembered-position fallback.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExArgumentTarget {
    Next(u64),
    Previous(u64),
    First,
    Last,
    /// A one-based file number, as written in `:argument`.
    Index(u64),
    Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentListError {
    Empty,
    BeforeFirst,
    AfterLast,
    InvalidIndex,
}

/// Return a zero-based destination without mutating the remembered position.
/// A matched current file wins over the position retained when `:edit` opened
/// an unrelated file. The caller commits that position only after opening the
/// selected file succeeds; splitting a view copies its remembered position.
pub fn resolve_argument(
    length: u64,
    current_index: Option<u64>,
    remembered_index: Option<u64>,
    target: ExArgumentTarget,
) -> Result<u64, ArgumentListError> {
    use ArgumentListError::*;
    if length == 0 {
        return Err(Empty);
    }
    let current = current_index.or(remembered_index).unwrap_or(0);
    if current >= length {
        return Err(InvalidIndex);
    }
    let destination = match target {
        ExArgumentTarget::Next(0) | ExArgumentTarget::Previous(0) | ExArgumentTarget::Index(0) => {
            return Err(InvalidIndex);
        }
        ExArgumentTarget::Next(count) => current.checked_add(count).ok_or(AfterLast)?,
        ExArgumentTarget::Previous(count) => current.checked_sub(count).ok_or(BeforeFirst)?,
        ExArgumentTarget::First => 0,
        ExArgumentTarget::Last => length - 1,
        ExArgumentTarget::Index(index) => index - 1,
        ExArgumentTarget::Current => current,
    };
    if destination >= length {
        return Err(AfterLast);
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_file_wins_and_replacement_uses_the_remembered_position() {
        assert_eq!(
            resolve_argument(5, Some(1), Some(3), ExArgumentTarget::Next(1)),
            Ok(2)
        );
        assert_eq!(
            resolve_argument(5, None, Some(3), ExArgumentTarget::Previous(2)),
            Ok(1)
        );
        assert_eq!(
            resolve_argument(5, None, Some(3), ExArgumentTarget::Current),
            Ok(3)
        );
        assert_eq!(
            resolve_argument(5, None, None, ExArgumentTarget::Next(1)),
            Ok(1)
        );
    }

    #[test]
    fn boundaries_counts_and_overflow_never_wrap() {
        assert_eq!(
            resolve_argument(0, None, None, ExArgumentTarget::First),
            Err(ArgumentListError::Empty)
        );
        assert_eq!(
            resolve_argument(3, Some(0), None, ExArgumentTarget::Previous(1)),
            Err(ArgumentListError::BeforeFirst)
        );
        assert_eq!(
            resolve_argument(3, Some(2), None, ExArgumentTarget::Next(1)),
            Err(ArgumentListError::AfterLast)
        );
        assert_eq!(
            resolve_argument(3, Some(1), None, ExArgumentTarget::Next(u64::MAX)),
            Err(ArgumentListError::AfterLast)
        );
        assert_eq!(
            resolve_argument(3, Some(1), None, ExArgumentTarget::Index(0)),
            Err(ArgumentListError::InvalidIndex)
        );
        assert_eq!(
            resolve_argument(3, Some(3), None, ExArgumentTarget::First),
            Err(ArgumentListError::InvalidIndex)
        );
        assert_eq!(
            resolve_argument(3, Some(1), None, ExArgumentTarget::First),
            Ok(0)
        );
        assert_eq!(
            resolve_argument(3, Some(1), None, ExArgumentTarget::Last),
            Ok(2)
        );
        assert_eq!(
            resolve_argument(3, Some(1), None, ExArgumentTarget::Index(3)),
            Ok(2)
        );
    }
}
