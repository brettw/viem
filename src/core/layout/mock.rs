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
    failure: Option<(usize, String)>,
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
        self.fail_next_batches(1, message);
    }

    pub fn fail_next_batches(&mut self, count: usize, message: impl Into<String>) {
        self.failure = (count > 0).then(|| (count, message.into()));
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
        TextMetrics {
            ascent: em * 0.78,
            descent: em * 0.22,
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
        Some(RenderRunHandle::new(policy.owner, identifier,
            request.metrics_generation, policy.threading))
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
        // Native shapers resolve an automatic paragraph direction from the
        // captured context. Keep the mock's run levels consistent with that
        // contract, including an explicit direction carried by a long-line
        // checkpoint into later fragments.
        let paragraph_base_direction = match request.paragraph_base_direction {
            TextDirection::Auto => {
                if unicode_bidi::get_base_direction(shaping_text.as_str())
                    == unicode_bidi::Direction::Rtl
                {
                    TextDirection::RightToLeft
                } else {
                    TextDirection::LeftToRight
                }
            }
            direction => direction,
        };
        let graphemes: Vec<(usize, &str)> = shaping_text.grapheme_indices(true).collect();
        let mut clusters = Vec::new();
        let mut fonts = std::collections::BTreeMap::<String, std::sync::Arc<str>>::new();
        let mut index = 0;

        while index < graphemes.len() {
            let local_start = graphemes[index].0;
            let global_start = context_start + local_start;
            let style = Self::style_at(request, global_start);
            let mut count = 1;

            // Every supported ligature starts with an uncombined `f`. Avoid
            // style-run searches and full style comparisons for other clusters,
            // which dominate the large literal-document test fixtures.
            if self.ligatures
                && graphemes[index].1 == "f"
                && Self::feature_enabled(style, *b"liga", true)
            {
                for candidate in [3, 2] {
                    if index + candidate <= graphemes.len() {
                        let candidate_end = if index + candidate == graphemes.len() {
                            shaping_text.len()
                        } else {
                            graphemes[index + candidate].0
                        };
                        let candidate_text = &shaping_text[local_start..candidate_end];
                        if !matches!(candidate_text, "fi" | "fl" | "ffi" | "ffl") {
                            continue;
                        }
                        let same_style = (index..index + candidate).all(|grapheme_index| {
                            let offset = context_start + graphemes[grapheme_index].0;
                            Self::style_at(request, offset) == style
                        });
                        if same_style {
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
                Self::bidi_level(cluster_text, style, paragraph_base_direction);
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
                    fallback_font: std::sync::Arc::clone(fonts
                        .entry(Self::fallback_font(cluster_text, style))
                        .or_insert_with_key(|name| std::sync::Arc::from(name.as_str()))),
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

        let visual_order = super::engine::fragment_visual_order(&clusters);
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
        if let Some((remaining, message)) = self.failure.take() {
            if remaining > 1 { self.failure = Some((remaining - 1, message.clone())); }
            return Err(MeasurementError::Provider(message));
        }
        Ok(requests
            .iter()
            .map(|request| self.shape_one(request))
            .collect())
    }
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
