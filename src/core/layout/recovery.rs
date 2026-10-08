//! Recovery changes disposable presentation, never source or edit semantics.
use super::{
    MeasurementError, ResolvedTextStyle, ShapeRequest, ShapedFragment, ShapingDiagnostic,
    TextMeasurementProvider,
};

/// Retry a declined native font/feature request once with ordinary system
/// typography. Identity, Unicode context, direction, scale and resource policy
/// remain exact. Malformed responses and unstable context are never repaired.
pub(super) fn shape_with_fallback<P: TextMeasurementProvider>(
    provider: &mut P,
    requests: &[ShapeRequest<'_>],
) -> Result<Vec<ShapedFragment>, MeasurementError> {
    let error = match provider.shape_batch(requests) {
        Ok(result) => return Ok(result),
        Err(error @ MeasurementError::Provider(_)) => error,
        Err(error) => return Err(error),
    };
    // A registration/device transition needs a new captured job, rather than
    // disguising incompatible resources as a font fallback.
    if requests.iter().any(|request| {
        request.metrics_generation != provider.metrics_generation()
            || request.measurement_environment_id != provider.measurement_environment_id()
    }) {
        return Err(error);
    }
    let fallback = ResolvedTextStyle::default();
    let defaults: Vec<_> = requests
        .iter()
        .map(|request| ResolvedTextStyle {
            direction: request.default_style.direction,
            language: request.default_style.language.clone(),
            script: request.default_style.script.clone(),
            ..fallback.clone()
        })
        .collect();
    let runs: Vec<Vec<_>> = requests
        .iter()
        .map(|request| {
            request
                .style_runs
                .iter()
                .map(|run| super::ShapeStyleRun {
                    text_range: run.text_range.clone(),
                    style: ResolvedTextStyle {
                        direction: run.style.direction,
                        language: run.style.language.clone(),
                        script: run.style.script.clone(),
                        ..fallback.clone()
                    },
                })
                .collect()
        })
        .collect();
    let fallback_requests: Vec<_> = requests
        .iter()
        .zip(&defaults)
        .zip(&runs)
        .map(|((request, style), runs)| ShapeRequest {
            default_style: style,
            style_runs: runs,
            ..request.clone()
        })
        .collect();
    let mut result = provider.shape_batch(&fallback_requests)?;
    let message = format!("Text shaping failed ({error:?}); using the default system font.");
    for (fragment, request) in result.iter_mut().zip(requests) {
        fragment.diagnostics.push(ShapingDiagnostic {
            text_range: request.text_range.clone(),
            message: message.chars().take(1024).collect(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocumentId, Revision};
    use crate::layout::*;

    struct FaultProvider {
        inner: MockTextMeasurementProvider,
        calls: usize,
        fault: u8,
    }
    impl TextMeasurementProvider for FaultProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.inner.measurement_environment_id()
        }
        fn metrics_generation(&self) -> MetricsGeneration {
            self.inner.metrics_generation()
        }
        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.inner.render_run_policy()
        }
        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            self.calls += 1;
            if self.fault == 2 {
                return Err(MeasurementError::Provider("device unavailable".into()));
            }
            if self.fault == 3 {
                self.inner.set_metrics_generation(MetricsGeneration(2));
                return Err(MeasurementError::Provider("device changed".into()));
            }
            if self.fault == 4 {
                return Err(MeasurementError::UnstableShapingContext(
                    "context insufficient".into(),
                ));
            }
            if self.fault == 6 {
                return Err(MeasurementError::InvalidResponse(
                    "invalid native output".into(),
                ));
            }
            if requests.iter().any(|r| {
                r.default_style.font_families != ResolvedTextStyle::default().font_families
            }) {
                return Err(MeasurementError::Provider("requested font failed".into()));
            }
            let mut result = self.inner.shape_batch(requests)?;
            if self.fault == 5 {
                result[0].document_revision = Revision(999);
            }
            Ok(result)
        }
    }
    fn provider(fault: u8) -> FaultProvider {
        FaultProvider {
            inner: MockTextMeasurementProvider::new(),
            calls: 0,
            fault,
        }
    }
    fn style() -> ResolvedTextStyle {
        ResolvedTextStyle {
            font_families: vec!["Broken font".into()],
            size: 31.0,
            language: Some("he".into()),
            script: Some("Hebr".into()),
            direction: TextDirection::RightToLeft,
            ..Default::default()
        }
    }

    #[test]
    fn font_failure_preserves_unicode_context_ranges_and_resource_identity() {
        let mut p = provider(1);
        let default = style();
        let runs = [ShapeStyleRun {
            text_range: 2..6,
            style: ResolvedTextStyle {
                direction: TextDirection::LeftToRight,
                language: Some("en".into()),
                ..default.clone()
            },
        }];
        let text = "אב e\u{301} 👩‍💻";
        let request = ShapeRequest {
            document_id: DocumentId(42),
            document_revision: Revision(7),
            measurement_environment_id: p.measurement_environment_id(),
            metrics_generation: p.metrics_generation(),
            text_range: 2..2 + text.len(),
            text,
            context_before: "a ",
            context_after: " z",
            style_runs: &runs,
            inline_images: &[],
            default_style: &default,
            paragraph_base_direction: TextDirection::RightToLeft,
            scale: 1.5,
            purpose: ShapePurpose::MetricsAndRenderData,
            render_run_policy: p.render_run_policy(),
        };
        let actual = shape_with_fallback(&mut p, &[request.clone()]).unwrap();
        let fallback = ResolvedTextStyle {
            direction: default.direction,
            language: default.language.clone(),
            script: default.script.clone(),
            ..Default::default()
        };
        let fallback_runs = [ShapeStyleRun {
            text_range: runs[0].text_range.clone(),
            style: ResolvedTextStyle {
                direction: runs[0].style.direction,
                language: runs[0].style.language.clone(),
                script: runs[0].style.script.clone(),
                ..Default::default()
            },
        }];
        let expected = p
            .inner
            .shape_batch(&[ShapeRequest {
                default_style: &fallback,
                style_runs: &fallback_runs,
                ..request
            }])
            .unwrap();
        assert_eq!(p.calls, 2);
        assert_eq!(actual[0].clusters, expected[0].clusters);
        assert_eq!(actual[0].visual_order, expected[0].visual_order);
        assert_eq!(actual[0].document_revision, Revision(7));
        assert_eq!(actual[0].text_range, 2..2 + text.len());
        assert!(actual[0].diagnostics[0]
            .message
            .contains("requested font failed"));
    }

    #[test]
    fn failures_have_a_finite_retry_budget_and_do_not_disguise_changed_resources() {
        for (fault, expected_calls) in [(2, 2), (3, 1), (4, 1), (6, 1)] {
            let mut engine = LayoutEngine::new(provider(fault));
            let mut view = ViewLayout::new(320.0, 200.0);
            view.set_default_style(style()).unwrap();
            assert!(engine
                .relayout_text(DocumentId(1), Revision(1), "hello", &mut view)
                .is_err());
            assert_eq!(engine.provider().calls, expected_calls);
            assert!(view.snapshot().is_none());
        }
    }

    #[test]
    fn malformed_fallback_response_is_rejected_without_installation() {
        let mut engine = LayoutEngine::new(provider(5));
        let mut view = ViewLayout::new(320.0, 200.0);
        view.set_default_style(style()).unwrap();
        assert!(engine
            .relayout_text(DocumentId(1), Revision(1), "hello", &mut view)
            .is_err());
        assert_eq!(engine.provider().calls, 2);
        assert!(view.snapshot().is_none());
    }

    #[test]
    fn recovery_is_cached_bounded_and_retires_when_metrics_change() {
        let mut engine = LayoutEngine::new(provider(1));
        let mut view = ViewLayout::new(320.0, 200.0);
        view.set_default_style(style()).unwrap();
        let text = "word e\u{301} 👩‍💻 אב\n".repeat(10_000);
        engine
            .relayout_text(DocumentId(1), Revision(1), &text, &mut view)
            .unwrap();
        assert!(!view.snapshot().unwrap().diagnostics.is_empty());
        let calls = engine.provider().calls;
        engine
            .relayout_text(DocumentId(1), Revision(1), &text, &mut view)
            .unwrap();
        assert_eq!(engine.provider().calls, calls);
        engine
            .provider_mut()
            .inner
            .set_metrics_generation(MetricsGeneration(2));
        engine
            .relayout_text(DocumentId(1), Revision(1), &text, &mut view)
            .unwrap();
        assert!(engine.provider().calls > calls);
        assert_eq!(
            view.snapshot().unwrap().metrics_generation,
            MetricsGeneration(2)
        );
        // Empty lines retain genuine caret geometry under recovery too.
        engine
            .relayout_text(DocumentId(1), Revision(2), "", &mut view)
            .unwrap();
        assert!(!view.snapshot().unwrap().rows.is_empty());
    }
}
