//! Portable review policy for changes observed in a document's backing file.
//!
//! Hosts detect changes and supply an opaque token identifying an observation.
//! No file I/O or platform identity interpretation belongs to this state.

pub const MAX_EXTERNAL_FILE_OBSERVATION_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalFileReview {
    pub can_reload: bool,
    pub discards_unsaved_changes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidExternalFileObservation;

#[derive(Debug, Default)]
pub struct ExternalFileReviewState {
    acknowledged: Option<Vec<u8>>,
    reviewing: Option<Vec<u8>>,
}

impl ExternalFileReviewState {
    /// A new binding or baseline invalidates prior observations and reviews.
    pub fn reset(&mut self) {
        self.acknowledged = None;
        self.reviewing = None;
    }

    /// Observing the saved baseline again ends the prior acknowledgement's
    /// lifetime. Preserve any open review; its host must validate the current
    /// observation before acknowledging the eventual decision.
    pub fn clear_acknowledged(&mut self) {
        self.acknowledged = None;
    }

    /// Start at most one review. Even a clean document requires an explicit
    /// decision before replacing its contents with externally changed bytes.
    pub fn begin(
        &mut self,
        token: &[u8],
        can_reload: bool,
        is_dirty: bool,
    ) -> Result<Option<ExternalFileReview>, InvalidExternalFileObservation> {
        validate_token(token)?;
        if self.reviewing.is_some() || self.acknowledged.as_deref() == Some(token) {
            return Ok(None);
        }
        self.reviewing = Some(token.to_vec());
        Ok(Some(ExternalFileReview {
            can_reload,
            discards_unsaved_changes: can_reload && is_dirty,
        }))
    }

    /// Finish only the matching review. Failed or stale actions release it
    /// without acknowledgement so the host can offer the observation again.
    /// A finish for an invalidated or different review changes nothing.
    pub fn finish(
        &mut self,
        token: &[u8],
        acknowledge: bool,
    ) -> Result<bool, InvalidExternalFileObservation> {
        validate_token(token)?;
        if self.reviewing.as_deref() != Some(token) {
            return Ok(false);
        }
        let reviewed = self.reviewing.take();
        if acknowledge {
            self.acknowledged = reviewed;
        }
        Ok(true)
    }
}

fn validate_token(token: &[u8]) -> Result<(), InvalidExternalFileObservation> {
    if token.is_empty() || token.len() > MAX_EXTERNAL_FILE_OBSERVATION_BYTES {
        return Err(InvalidExternalFileObservation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_and_dirty_documents_require_review_with_correct_reload_warning() {
        for can_reload in [false, true] {
            for is_dirty in [false, true] {
                let mut state = ExternalFileReviewState::default();
                assert_eq!(
                    state.begin(b"changed", can_reload, is_dirty),
                    Ok(Some(ExternalFileReview {
                        can_reload,
                        discards_unsaved_changes: can_reload && is_dirty,
                    }))
                );
            }
        }
    }

    #[test]
    fn acknowledgement_deduplicates_only_the_same_observation() {
        let mut state = ExternalFileReviewState::default();
        assert!(state.begin(b"first", true, false).unwrap().is_some());
        assert_eq!(state.finish(b"first", true), Ok(true));
        assert_eq!(state.begin(b"first", true, true), Ok(None));
        assert!(state.begin(b"second", true, true).unwrap().is_some());
        assert_eq!(state.finish(b"second", true), Ok(true));
        // Returning to an older disk state is itself a new change.
        assert!(state.begin(b"first", true, false).unwrap().is_some());
    }

    #[test]
    fn only_one_review_is_active_and_wrong_completion_cannot_release_it() {
        let mut state = ExternalFileReviewState::default();
        assert!(state.begin(b"first", true, false).unwrap().is_some());
        assert_eq!(state.begin(b"first", true, false), Ok(None));
        assert_eq!(state.begin(b"second", true, false), Ok(None));
        assert_eq!(state.finish(b"second", true), Ok(false));
        assert_eq!(state.begin(b"second", true, false), Ok(None));
        assert_eq!(state.finish(b"first", true), Ok(true));
        assert!(state.begin(b"second", true, false).unwrap().is_some());
    }

    #[test]
    fn returning_to_baseline_allows_reviewing_same_external_bytes_again() {
        let mut state = ExternalFileReviewState::default();
        state.begin(b"changed", true, false).unwrap();
        state.finish(b"changed", true).unwrap();
        assert_eq!(state.begin(b"changed", true, false), Ok(None));
        state.clear_acknowledged();
        assert!(state.begin(b"changed", true, false).unwrap().is_some());
    }

    #[test]
    fn clearing_acknowledgement_preserves_pending_review_and_allows_stale_release() {
        let mut state = ExternalFileReviewState::default();
        state.begin(b"first", true, false).unwrap();
        state.finish(b"first", true).unwrap();
        state.begin(b"second", true, false).unwrap();
        state.clear_acknowledged();
        assert_eq!(state.begin(b"first", true, false), Ok(None));
        assert_eq!(state.begin(b"second", true, false), Ok(None));
        // The host observed the baseline while this prompt was open, so its
        // now-stale decision must release the review without acknowledging.
        assert_eq!(state.finish(b"second", false), Ok(true));
        assert!(state.begin(b"first", true, false).unwrap().is_some());
        state.finish(b"first", false).unwrap();
        assert!(state.begin(b"second", true, false).unwrap().is_some());
    }

    #[test]
    fn unsuccessful_action_allows_retry_without_losing_prior_acknowledgement() {
        let mut state = ExternalFileReviewState::default();
        state.begin(b"first", true, false).unwrap();
        state.finish(b"first", true).unwrap();
        state.begin(b"second", true, false).unwrap();
        assert_eq!(state.finish(b"second", false), Ok(true));
        assert_eq!(state.begin(b"first", true, false), Ok(None));
        assert!(state.begin(b"second", true, false).unwrap().is_some());
    }

    #[test]
    fn reset_invalidates_pending_and_acknowledged_observations() {
        let mut state = ExternalFileReviewState::default();
        state.begin(b"first", true, false).unwrap();
        state.finish(b"first", true).unwrap();
        state.begin(b"second", true, false).unwrap();
        state.reset();
        assert_eq!(state.finish(b"second", true), Ok(false));
        assert!(state.begin(b"first", true, false).unwrap().is_some());
        state.finish(b"first", true).unwrap();
        assert!(state.begin(b"second", true, false).unwrap().is_some());
    }

    #[test]
    fn tokens_are_opaque_and_bounded_and_rejection_preserves_state() {
        let mut state = ExternalFileReviewState::default();
        let token = [0, 255, 0, 128];
        assert!(state.begin(&token, true, false).unwrap().is_some());
        assert_eq!(state.finish(&[], true), Err(InvalidExternalFileObservation));
        let too_large = vec![0; MAX_EXTERNAL_FILE_OBSERVATION_BYTES + 1];
        assert_eq!(
            state.begin(&too_large, true, false),
            Err(InvalidExternalFileObservation)
        );
        assert_eq!(state.finish(&token, true), Ok(true));
        assert_eq!(state.begin(&token, true, false), Ok(None));
        let largest = vec![1; MAX_EXTERNAL_FILE_OBSERVATION_BYTES];
        assert!(state.begin(&largest, true, false).unwrap().is_some());
    }
}
