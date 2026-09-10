//! Deterministic shaper used by portable core tests.

use super::measurement::*;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub struct MockTextMeasurementProvider {
    measurement_environment_id: MeasurementEnvironmentId,
    generation: MetricsGeneration,
    render_policy: RenderRunPolicy,
    batch_calls: usize,
    request_calls: usize,
    ligatures: bool,
    response_generation_override: Option<MetricsGeneration>,
    failure: Option<String>,
}

impl Default for MockTextMeasurementProvider {
    fn default() -> Self {
        Self {
            measurement_environment_id: MeasurementEnvironmentId::default(),
            generation: MetricsGeneration(1),
            render_policy: Self::default_render_policy(),
            batch_calls: 0,
            request_calls: 0,
            ligatures: true,
            response_generation_override: None,
            failure: None,
        }
    }
}

impl MockTextMeasurementProvider {
    const RENDER_OWNER: RenderRunOwner = RenderRunOwner(0x4d4f_434b);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_metrics_generation(&mut self, generation: MetricsGeneration) {
        self.generation = generation;
    }

    pub fn set_measurement_environment_id(
        &mut self,
        measurement_environment_id: MeasurementEnvironmentId,
    ) {
        self.measurement_environment_id = measurement_environment_id;
    }

    pub fn set_ligatures(&mut self, enabled: bool) {
        self.ligatures = enabled;
    }

    pub fn set_response_generation_override(&mut self, generation: Option<MetricsGeneration>) {
        self.response_generation_override = generation;
    }

    pub fn fail_next_batch(&mut self, message: impl Into<String>) {
        self.failure = Some(message.into());
    }

    pub fn batch_calls(&self) -> usize {
        self.batch_calls
    }

    pub fn request_calls(&self) -> usize {
        self.request_calls
    }

    fn default_render_policy() -> RenderRunPolicy {
        RenderRunPolicy {
            owner: Self::RENDER_OWNER,
            threading: RenderRunThreading::AnyThread,
        }
    }

    pub fn set_render_run_policy(&mut self, policy: RenderRunPolicy) {
        self.render_policy = policy;
    }

    fn style_at<'a>(request: &'a ShapeRequest<'_>, offset: usize) -> &'a ResolvedTextStyle {
        request
            .style_runs
            .iter()
            .find(|run| run.text_range.start <= offset && offset < run.text_range.end)
            .map_or(request.default_style, |run| &run.style)
    }

    fn scalar_width(character: char) -> f32 {
        match character {
            '\t' => 4.0,
            ' ' | '\u{00a0}' => 0.5,
            '\u{200b}' | '\u{200d}' | '\u{2060}' | '\u{fe0e}' | '\u{fe0f}' => 0.0,
            c if is_combining_mark(c) => 0.0,
            'i' | 'l' | 'I' | '!' | '.' | ',' | ':' | ';' | '\'' => 0.45,
            'm' | 'w' | 'M' | 'W' => 0.95,
            c if is_emoji(c) || is_cjk(c) => 1.8,
            c if c.is_ascii() => 0.75,
            _ => 0.9,
        }
    }

    fn metrics(style: &ResolvedTextStyle, scale: f32) -> TextMetrics {
        let em = style.size * scale;
        let baseline_shift = style.baseline_shift * scale;
        TextMetrics {
            ascent: em * 0.78 + baseline_shift.max(0.0),
            descent: em * 0.22 + (-baseline_shift).max(0.0),
            leading: em * 0.12,
        }
    }

    fn cluster_advance(text: &str, style: &ResolvedTextStyle, scale: f32) -> f32 {
        let em = style.size * scale;
        let mut width = text.chars().map(Self::scalar_width).sum::<f32>();
        if text == "fi" || text == "fl" {
            width *= 0.88;
        } else if text == "ffi" || text == "ffl" {
            width *= 0.82;
        }
        width * em + style.letter_spacing * scale
    }

    fn feature_enabled(style: &ResolvedTextStyle, tag: [u8; 4], default: bool) -> bool {
        style
            .features
            .iter()
            .find(|feature| feature.tag == tag)
            .map_or(default, |feature| feature.value != 0)
    }

    fn kerning_adjustment(
        current: &str,
        next: Option<(&str, &ResolvedTextStyle)>,
        style: &ResolvedTextStyle,
        scale: f32,
    ) -> f32 {
        let Some((next, next_style)) = next else {
            return 0.0;
        };
        if style != next_style {
            return 0.0;
        }
        let pair = current.chars().last().zip(next.chars().next());
        if matches!(
            pair,
            Some(('A', 'V' | 'W' | 'Y') | ('T', 'a' | 'o') | ('W', 'a') | ('Y', 'o'))
        ) {
            -0.12 * style.size * scale
        } else {
            0.0
        }
    }

    fn bidi_level(
        text: &str,
        style: &ResolvedTextStyle,
        paragraph_base_direction: TextDirection,
    ) -> u8 {
        let paragraph_is_rtl = paragraph_base_direction == TextDirection::RightToLeft;
        match style.direction {
            TextDirection::LeftToRight => u8::from(paragraph_is_rtl) * 2,
            TextDirection::RightToLeft => 1,
            TextDirection::Auto if text.chars().any(is_rtl) => 1,
            TextDirection::Auto => u8::from(paragraph_is_rtl) * 2,
        }
    }

    fn bounds(
        advance: f32,
        metrics: &TextMetrics,
        style: &ResolvedTextStyle,
        scale: f32,
    ) -> (ShapedBounds, ShapedBounds) {
        let typographic = ShapedBounds {
            x: 0.0,
            y: -metrics.ascent,
            width: advance,
            height: metrics.ascent + metrics.descent,
        };
        let overhang = if style.slant == FontSlant::Upright {
            0.0
        } else {
            style.size * scale * 0.06
        };
        let ink = ShapedBounds {
            x: -overhang,
            y: -metrics.ascent,
            width: advance + overhang * 2.0,
            height: metrics.ascent + metrics.descent,
        };
        (typographic, ink)
    }

    fn render_run(
        request: &ShapeRequest<'_>,
        previous: Option<&str>,
        text: &str,
        next: Option<&str>,
        style: &ResolvedTextStyle,
    ) -> Option<RenderRunHandle> {
        if request.purpose == ShapePurpose::MetricsOnly {
            return None;
        }
        let policy = request.render_run_policy?;
        // A render run is relative and safe to reuse at another formatted
        // offset. Keep this identifier content/style/generation-derived rather
        // than tying it to a document ordinal.
        let mut identifier = 0xcbf2_9ce4_8422_2325_u64;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                identifier ^= u64::from(*byte);
                identifier = identifier.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(&[0x01]);
        if let Some(previous) = previous {
            mix(previous.as_bytes());
        }
        mix(&[0x02]);
        mix(text.as_bytes());
        mix(&[0x03]);
        if let Some(next) = next {
            mix(next.as_bytes());
        }
        mix(&[0x04]);
        for family in &style.font_families {
            mix(family.as_bytes());
            mix(&[0]);
        }
        mix(&style.size.to_bits().to_le_bytes());
        mix(&style.weight.to_bits().to_le_bytes());
        mix(&[u8::from(style.relative_bold)]);
        mix(&style.letter_spacing.to_bits().to_le_bytes());
        mix(&style.baseline_shift.to_bits().to_le_bytes());
        mix(&request.scale.to_bits().to_le_bytes());
        mix(&request.metrics_generation.0.to_le_bytes());
        mix(&request.measurement_environment_id.0.to_le_bytes());
        mix(&[match request.paragraph_base_direction {
            TextDirection::Auto => 0,
            TextDirection::LeftToRight => 1,
            TextDirection::RightToLeft => 2,
        }]);
        mix(&[match style.slant {
            FontSlant::Upright => 0,
            FontSlant::Italic => 1,
            FontSlant::Oblique => 2,
        }]);
        mix(&[match style.direction {
            TextDirection::Auto => 0,
            TextDirection::LeftToRight => 1,
            TextDirection::RightToLeft => 2,
        }]);
        if let Some(language) = &style.language {
            mix(language.as_bytes());
        }
        mix(&[0]);
        if let Some(script) = &style.script {
            mix(script.as_bytes());
        }
        for feature in &style.features {
            mix(&feature.tag);
            mix(&feature.value.to_le_bytes());
        }
        Some(RenderRunHandle {
            owner: policy.owner,
            identifier,
            metrics_generation: request.metrics_generation,
            threading: policy.threading,
        })
    }

    fn fallback_font(text: &str, style: &ResolvedTextStyle) -> String {
        if text.chars().any(is_emoji) {
            "Mock Emoji".to_owned()
        } else if text.chars().any(|character| !character.is_ascii()) {
            style
                .font_families
                .get(1)
                .cloned()
                .unwrap_or_else(|| "Mock Fallback".to_owned())
        } else {
            style
                .font_families
                .first()
                .cloned()
                .unwrap_or_else(|| "Mock Sans".to_owned())
        }
    }

    fn shape_one(&self, request: &ShapeRequest<'_>) -> ShapedFragment {
        let default_style = request.default_style.clone().with_implicit_kerning();
        let style_runs = request
            .style_runs
            .iter()
            .cloned()
            .map(|run| ShapeStyleRun {
                style: run.style.with_implicit_kerning(),
                ..run
            })
            .collect::<Vec<_>>();
        let request = &ShapeRequest {
            default_style: &default_style,
            style_runs: &style_runs,
            ..request.clone()
        };
        let context_start = request
            .text_range
            .start
            .checked_sub(request.context_before.len())
            .expect("mock shaping context must have valid global coordinates");
        let mut shaping_text = String::with_capacity(
            request.context_before.len() + request.text.len() + request.context_after.len(),
        );
        shaping_text.push_str(request.context_before);
        shaping_text.push_str(request.text);
        shaping_text.push_str(request.context_after);
        let graphemes: Vec<(usize, &str)> = shaping_text.grapheme_indices(true).collect();
        let mut clusters = Vec::new();
        let mut index = 0;

        while index < graphemes.len() {
            let local_start = graphemes[index].0;
            let global_start = context_start + local_start;
            let style = Self::style_at(request, global_start);
            let mut count = 1;

            if self.ligatures && Self::feature_enabled(style, *b"liga", true) {
                for candidate in [3, 2] {
                    if index + candidate <= graphemes.len() {
                        let candidate_end = if index + candidate == graphemes.len() {
                            shaping_text.len()
                        } else {
                            graphemes[index + candidate].0
                        };
                        let candidate_text = &shaping_text[local_start..candidate_end];
                        let same_style = (index..index + candidate).all(|grapheme_index| {
                            let offset = context_start + graphemes[grapheme_index].0;
                            Self::style_at(request, offset) == style
                        });
                        if same_style && matches!(candidate_text, "fi" | "fl" | "ffi" | "ffl") {
                            count = candidate;
                            break;
                        }
                    }
                }
            }

            let local_end = if index + count == graphemes.len() {
                shaping_text.len()
            } else {
                graphemes[index + count].0
            };
            let global_end = context_start + local_end;
            let cluster_text = &shaping_text[local_start..local_end];
            let previous = if index > 0 {
                let previous_start = graphemes[index - 1].0;
                Some(&shaping_text[previous_start..local_start])
            } else {
                None
            };
            let next = if index + count < graphemes.len() {
                let next_start = graphemes[index + count].0;
                let next_end = if index + count + 1 == graphemes.len() {
                    shaping_text.len()
                } else {
                    graphemes[index + count + 1].0
                };
                let next_global = context_start + next_start;
                Some((
                    &shaping_text[next_start..next_end],
                    Self::style_at(request, next_global),
                ))
            } else {
                None
            };
            let advance = (Self::cluster_advance(cluster_text, style, request.scale)
                + Self::kerning_adjustment(cluster_text, next, style, request.scale))
            .max(0.0);
            let bidi_level =
                Self::bidi_level(cluster_text, style, request.paragraph_base_direction);
            let metrics = Self::metrics(style, request.scale);
            let (typographic_bounds, ink_bounds) =
                Self::bounds(advance, &metrics, style, request.scale);
            let (start_inline, end_inline) = if bidi_level % 2 == 0 {
                (0.0, advance)
            } else {
                (advance, 0.0)
            };

            if request.text_range.start <= global_start && global_start < request.text_range.end {
                clusters.push(ShapedCluster {
                    text_range: global_start..global_end,
                    advance,
                    metrics,
                    typographic_bounds,
                    ink_bounds,
                    bidi_level,
                    fallback_font: Self::fallback_font(cluster_text, style),
                    caret_stops: vec![
                        ClusterCaretStop {
                            text_offset: global_start,
                            inline_offset: start_inline,
                            affinity: BoundaryAffinity::Downstream,
                        },
                        ClusterCaretStop {
                            text_offset: global_end,
                            inline_offset: end_inline,
                            affinity: BoundaryAffinity::Upstream,
                        },
                    ],
                    render_run: Self::render_run(
                        request,
                        previous,
                        cluster_text,
                        next.map(|(text, _)| text),
                        style,
                    ),
                });
            }
            index += count;
        }

        let visual_order = visual_reorder(&clusters);
        let default_metrics = Self::metrics(request.default_style, request.scale);
        ShapedFragment {
            document_id: request.document_id,
            document_revision: request.document_revision,
            measurement_environment_id: request.measurement_environment_id,
            metrics_generation: self
                .response_generation_override
                .unwrap_or(request.metrics_generation),
            text_range: request.text_range.clone(),
            clusters,
            visual_order,
            default_metrics,
            diagnostics: Vec::new(),
        }
    }
}

impl TextMeasurementProvider for MockTextMeasurementProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    fn metrics_generation(&self) -> MetricsGeneration {
        self.generation
    }

    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        Some(self.render_policy)
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.batch_calls += 1;
        self.request_calls += requests.len();
        if let Some(message) = self.failure.take() {
            return Err(MeasurementError::Provider(message));
        }
        Ok(requests
            .iter()
            .map(|request| self.shape_one(request))
            .collect())
    }
}

fn visual_reorder(clusters: &[ShapedCluster]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..clusters.len()).collect();
    let max_level = clusters
        .iter()
        .map(|cluster| cluster.bidi_level)
        .max()
        .unwrap_or(0);
    let Some(min_odd_level) = clusters
        .iter()
        .map(|cluster| cluster.bidi_level)
        .filter(|level| level % 2 == 1)
        .min()
    else {
        return order;
    };

    for level in (min_odd_level..=max_level).rev() {
        let mut start = 0;
        while start < order.len() {
            while start < order.len() && clusters[order[start]].bidi_level < level {
                start += 1;
            }
            let mut end = start;
            while end < order.len() && clusters[order[end]].bidi_level >= level {
                end += 1;
            }
            order[start..end].reverse();
            start = end;
        }
    }
    order
}

pub(crate) fn is_rtl(character: char) -> bool {
    matches!(
        character as u32,
        0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff | 0x10800..=0x10fff
    )
}

fn is_combining_mark(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
            | 0xfe00..=0xfe0f
            | 0xe0100..=0xe01ef
    )
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x2ffff
    )
}

fn is_emoji(character: char) -> bool {
    matches!(character as u32, 0x1f000..=0x1faff | 0x2600..=0x27bf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocumentId, Revision};

    fn request<'a>(
        text: &'a str,
        style: &'a ResolvedTextStyle,
        purpose: ShapePurpose,
    ) -> ShapeRequest<'a> {
        ShapeRequest {
            document_id: DocumentId(7),
            document_revision: Revision(11),
            measurement_environment_id: MeasurementEnvironmentId::default(),
            text_range: 20..20 + text.len(),
            text,
            context_before: "",
            context_after: "",
            style_runs: &[],
            default_style: style,
            paragraph_base_direction: TextDirection::Auto,
            scale: 1.0,
            metrics_generation: MetricsGeneration(1),
            purpose,
            render_run_policy: (purpose == ShapePurpose::MetricsAndRenderData)
                .then_some(MockTextMeasurementProvider::default_render_policy()),
        }
    }

    #[test]
    fn deterministic_batch_preserves_revisions_fallback_and_visual_order() {
        let mut provider = MockTextMeasurementProvider::new();
        let style = ResolvedTextStyle {
            font_families: vec!["Primary".to_owned(), "Unicode Fallback".to_owned()],
            ..ResolvedTextStyle::default()
        };
        let text = "aאב🙂";
        let request = request(text, &style, ShapePurpose::MetricsAndRenderData);
        let response = provider.shape_batch(&[request]).unwrap().remove(0);
        assert_eq!(response.document_id, DocumentId(7));
        assert_eq!(response.document_revision, Revision(11));
        assert_eq!(
            response.measurement_environment_id,
            MeasurementEnvironmentId::default()
        );
        assert_eq!(response.metrics_generation, MetricsGeneration(1));
        assert_eq!(response.visual_order, vec![0, 2, 1, 3]);
        assert_eq!(response.clusters[1].fallback_font, "Unicode Fallback");
        assert_eq!(response.clusters[3].fallback_font, "Mock Emoji");
        assert!(response.clusters.iter().all(|cluster| {
            cluster.render_run.is_some_and(|run| {
                run.owner == RenderRunOwner(0x4d4f_434b)
                    && run.metrics_generation == MetricsGeneration(1)
                    && run.threading == RenderRunThreading::AnyThread
            })
        }));
        assert_eq!(provider.batch_calls(), 1);
        assert_eq!(provider.request_calls(), 1);
    }

    #[test]
    fn kerning_ligatures_features_context_and_combining_clusters_are_deterministic() {
        let base = ResolvedTextStyle::default();
        let mut provider = MockTextMeasurementProvider::new();

        let pair = provider
            .shape_batch(&[request("AV", &base, ShapePurpose::MetricsAndRenderData)])
            .unwrap()
            .remove(0);
        let isolated = provider
            .shape_batch(&[request("AX", &base, ShapePurpose::MetricsAndRenderData)])
            .unwrap()
            .remove(0);
        assert!(pair.clusters[0].advance < isolated.clusters[0].advance);

        let context_request = ShapeRequest {
            text_range: 20..21,
            text: "A",
            context_after: "V",
            ..request("A", &base, ShapePurpose::MetricsAndRenderData)
        };
        let contextual = provider
            .shape_batch(&[context_request.clone()])
            .unwrap()
            .remove(0);
        assert_eq!(contextual.clusters[0].advance, pair.clusters[0].advance);
        assert_eq!(
            provider.shape_batch(&[context_request]).unwrap().remove(0),
            contextual
        );

        let leading_context_request = ShapeRequest {
            text_range: 21..22,
            text: "V",
            context_before: "A",
            ..request("V", &base, ShapePurpose::MetricsAndRenderData)
        };
        let leading_context = provider
            .shape_batch(&[leading_context_request.clone()])
            .unwrap()
            .remove(0);
        assert_eq!(
            leading_context.clusters[0].render_run, pair.clusters[1].render_run,
            "leading context must preserve the same shaped render identity"
        );
        let different_leading_context = provider
            .shape_batch(&[ShapeRequest {
                context_before: "X",
                ..leading_context_request
            }])
            .unwrap()
            .remove(0);
        assert_ne!(
            different_leading_context.clusters[0].render_run,
            leading_context.clusters[0].render_run,
            "the mock must make context-before dependencies observable"
        );

        let ligature_head_request = ShapeRequest {
            text_range: 20..21,
            text: "f",
            context_after: "i",
            ..request("f", &base, ShapePurpose::MetricsAndRenderData)
        };
        let ligature_head = provider
            .shape_batch(&[ligature_head_request])
            .unwrap()
            .remove(0);
        assert_eq!(ligature_head.clusters.len(), 1);
        assert_eq!(ligature_head.clusters[0].text_range, 20..22);

        let ligature_tail_request = ShapeRequest {
            text_range: 21..22,
            text: "i",
            context_before: "f",
            ..request("i", &base, ShapePurpose::MetricsAndRenderData)
        };
        let ligature_tail = provider
            .shape_batch(&[ligature_tail_request])
            .unwrap()
            .remove(0);
        assert!(
            ligature_tail.clusters.is_empty(),
            "the adjacent fragment must omit a cluster owned by preceding context"
        );

        let features_off = ResolvedTextStyle {
            features: vec![
                OpenTypeFeature {
                    tag: *b"kern",
                    value: 0,
                },
                OpenTypeFeature {
                    tag: *b"liga",
                    value: 0,
                },
            ],
            ..base.clone()
        };
        let still_kerned = provider
            .shape_batch(&[request(
                "AV",
                &features_off,
                ShapePurpose::MetricsAndRenderData,
            )])
            .unwrap()
            .remove(0);
        assert_eq!(still_kerned.clusters[0].advance, pair.clusters[0].advance);

        let ignored_kerning = ResolvedTextStyle {
            features: vec![OpenTypeFeature {
                tag: *b"kern",
                value: 0,
            }],
            ..base.clone()
        };
        assert_eq!(
            provider
                .shape_batch(&[request(
                    "AV",
                    &ignored_kerning,
                    ShapePurpose::MetricsAndRenderData
                )])
                .unwrap()
                .remove(0),
            pair,
            "implicit kerning also preserves the render identity",
        );

        let ligature = provider
            .shape_batch(&[request("ffi", &base, ShapePurpose::MetricsAndRenderData)])
            .unwrap()
            .remove(0);
        let no_ligature = provider
            .shape_batch(&[request(
                "ffi",
                &features_off,
                ShapePurpose::MetricsAndRenderData,
            )])
            .unwrap()
            .remove(0);
        assert_eq!(ligature.clusters.len(), 1);
        assert_eq!(ligature.clusters[0].caret_stops.len(), 2);
        assert_eq!(no_ligature.clusters.len(), 3);

        let combining = provider
            .shape_batch(&[request(
                "e\u{301}",
                &base,
                ShapePurpose::MetricsAndRenderData,
            )])
            .unwrap()
            .remove(0);
        let plain = provider
            .shape_batch(&[request("e", &base, ShapePurpose::MetricsAndRenderData)])
            .unwrap()
            .remove(0);
        assert_eq!(combining.clusters.len(), 1);
        assert_eq!(combining.clusters[0].advance, plain.clusters[0].advance);
    }

    #[test]
    fn direction_bounds_render_purpose_and_empty_metrics_are_explicit() {
        let right_to_left = ResolvedTextStyle {
            direction: TextDirection::RightToLeft,
            slant: FontSlant::Italic,
            ..ResolvedTextStyle::default()
        };
        let mut provider = MockTextMeasurementProvider::new();
        let rendered = provider
            .shape_batch(&[request(
                "ab",
                &right_to_left,
                ShapePurpose::MetricsAndRenderData,
            )])
            .unwrap()
            .remove(0);
        assert_eq!(rendered.visual_order, vec![1, 0]);
        assert!(rendered.clusters.iter().all(|cluster| {
            cluster.bidi_level == 1
                && cluster.typographic_bounds.width == cluster.advance
                && cluster.ink_bounds.x < 0.0
                && cluster.ink_bounds.width > cluster.typographic_bounds.width
                && cluster.render_run.is_some()
        }));

        let metrics_only = provider
            .shape_batch(&[request("ab", &right_to_left, ShapePurpose::MetricsOnly)])
            .unwrap()
            .remove(0);
        assert!(metrics_only
            .clusters
            .iter()
            .all(|cluster| cluster.render_run.is_none()));

        let empty = provider
            .shape_batch(&[request("", &right_to_left, ShapePurpose::MetricsOnly)])
            .unwrap()
            .remove(0);
        assert!(empty.clusters.is_empty());
        assert!(empty.default_metrics.is_valid());
    }

    #[test]
    fn paragraph_base_direction_is_independent_from_character_direction() {
        let style = ResolvedTextStyle::default();
        let mut provider = MockTextMeasurementProvider::new();
        let left_to_right = provider
            .shape_batch(&[request("ab", &style, ShapePurpose::MetricsAndRenderData)])
            .unwrap()
            .remove(0);
        let right_to_left = provider
            .shape_batch(&[ShapeRequest {
                paragraph_base_direction: TextDirection::RightToLeft,
                ..request("ab", &style, ShapePurpose::MetricsAndRenderData)
            }])
            .unwrap()
            .remove(0);

        assert!(left_to_right
            .clusters
            .iter()
            .all(|cluster| cluster.bidi_level == 0));
        assert!(right_to_left
            .clusters
            .iter()
            .all(|cluster| cluster.bidi_level == 2));
        assert_ne!(
            left_to_right.clusters[0].render_run, right_to_left.clusters[0].render_run,
            "paragraph direction participates in render-data identity"
        );
    }

    #[test]
    fn injected_failure_is_consumed_by_one_batch() {
        let mut provider = MockTextMeasurementProvider::new();
        provider.fail_next_batch("planned failure");
        assert_eq!(
            provider.shape_batch(&[]),
            Err(MeasurementError::Provider("planned failure".to_owned()))
        );
        assert_eq!(provider.shape_batch(&[]), Ok(Vec::new()));
    }
}
