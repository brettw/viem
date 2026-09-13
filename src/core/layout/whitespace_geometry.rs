//! Portable advances for whitespace, applied after shaping and before wrapping.
use super::*;

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    /// Measure layout-only margin units with the hard line's original font
    /// context. The shaped space stays in the measurement cache, not the row.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_wrap_extra_indent(
        &mut self,
        line_start: usize,
        view: &LayoutJobViewConfiguration,
        whitespace_style: &ResolvedTextStyle,
        default_style: &ResolvedTextStyle,
        style_runs: &[ShapeStyleRun],
        document_id: DocumentId,
        revision: Revision,
        control: &LayoutRunControl<'_>,
    ) -> Result<f32, LayoutComputationError> {
        if view.whitespace.options.code_wrapped_line_indent == 0 {
            return Ok(0.0);
        }
        let style = if view.whitespace.basis() == super::super::WhitespaceBasis::ParagraphEn {
            whitespace_style
        } else {
            let index = style_runs.partition_point(|run| run.text_range.end <= line_start);
            style_runs
                .get(index)
                .filter(|run| run.text_range.start <= line_start)
                .map_or(default_style, |run| &run.style)
        };
        Ok(self.whitespace_unit(
            &view.whitespace,
            style,
            view.scale,
            document_id,
            revision,
            control,
        )? * view.whitespace.options.code_wrapped_line_indent as f32)
    }

    /// A huge tab prefix may span several independently wrapped slices. Read
    /// and shape only that prefix in bounded fragments once; its total margin
    /// is then carried by the ordinary exact long-line checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_leading_indent_from_tree(
        &mut self,
        tree: &crate::document::FormattedTextTree,
        line: &Range<usize>,
        view: &LayoutJobViewConfiguration,
        whitespace_unit: f32,
        default_style: &ResolvedTextStyle,
        style_runs: &[ShapeStyleRun],
        document_id: DocumentId,
        revision: Revision,
        control: &LayoutRunControl<'_>,
    ) -> Result<(f64, LayoutWorkStatistics), LayoutComputationError> {
        let mut prefix_end = line.start;
        while prefix_end < line.end {
            control.checkpoint()?;
            let chunk = tree.byte_chunk_at(prefix_end);
            let chunk = &chunk[..chunk
                .len()
                .min(line.end - prefix_end)
                .min(MAX_SHAPE_FRAGMENT_BYTES)];
            let count = chunk
                .iter()
                .take_while(|&&byte| byte == b' ' || byte == b'\t')
                .count();
            prefix_end += count;
            if count == 0 || count < chunk.len() {
                break;
            }
        }
        if !tree
            .is_grapheme_boundary(prefix_end)
            .map_err(|_| LayoutError::InvalidTextOffset(prefix_end))?
        {
            prefix_end = tree
                .previous_grapheme_boundary(prefix_end)
                .map_err(|_| LayoutError::InvalidTextOffset(prefix_end))?
                .ok_or(LayoutError::InvalidTextOffset(prefix_end))?;
        }
        let mut at = line.start;
        let mut advance = 0.0;
        let mut statistics = LayoutWorkStatistics::default();
        while at < prefix_end {
            control.checkpoint()?;
            let end = (at + MAX_SHAPE_FRAGMENT_BYTES).min(prefix_end);
            let mut fragment = self.shape_unwrapped_fragment(
                document_id,
                revision,
                tree,
                at..end,
                line,
                default_style,
                style_runs,
                TextDirection::LeftToRight,
                view,
                control,
            )?;
            let text = tree
                .slice(at..end)
                .map_err(|_| LayoutError::InvalidTextOffset(at))?;
            (_, advance, _) = self.layout_whitespace_clusters(
                &mut fragment.clusters,
                &text,
                at,
                &view.whitespace,
                whitespace_unit,
                default_style,
                style_runs,
                view.scale,
                document_id,
                revision,
                control,
                true,
                advance,
            )?;
            statistics.segmented_text_bytes += end - at;
            statistics.shaping_fragment_count += 1;
            statistics.maximum_shaping_fragment_bytes =
                statistics.maximum_shaping_fragment_bytes.max(end - at);
            at = end;
        }
        Ok((advance, statistics))
    }

    pub(super) fn whitespace_unit(
        &mut self,
        config: &WhitespaceConfiguration,
        style: &ResolvedTextStyle,
        scale: f32,
        document_id: DocumentId,
        revision: Revision,
        control: &LayoutRunControl<'_>,
    ) -> Result<f32, LayoutComputationError> {
        if config.basis() == super::super::WhitespaceBasis::ParagraphEn {
            return Ok(style.size * scale * 0.5);
        }
        // Spaces mode follows the tab's effective character font and tracking.
        // Reuse the bounded furniture shape cache for repeated font instances.
        let style = style.clone();
        std::mem::swap(&mut self.shape_cache, &mut self.decoration_shape_cache);
        let result = self.shape_ranges_with_origin(
            document_id,
            revision,
            " ",
            &[0..1],
            &[0..1],
            0,
            &[],
            &[style],
            &[TextDirection::LeftToRight],
            scale,
            self.provider.measurement_environment_id(),
            self.provider.metrics_generation(),
            control,
        );
        std::mem::swap(&mut self.shape_cache, &mut self.decoration_shape_cache);
        let width: f32 = result?
            .iter()
            .flat_map(|f| &f.clusters)
            .map(|c| c.advance)
            .sum();
        if width.is_finite() && width > 0.0 {
            Ok(width)
        } else {
            Err(LayoutError::MalformedMeasurement(
                "the current font's space requires positive advance",
            )
            .into())
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn layout_whitespace_clusters(
        &mut self,
        clusters: &mut [ShapedCluster],
        text: &str,
        origin: usize,
        config: &WhitespaceConfiguration,
        paragraph_unit: f32,
        default_style: &ResolvedTextStyle,
        style_runs: &[ShapeStyleRun],
        scale: f32,
        document_id: DocumentId,
        revision: Revision,
        control: &LayoutRunControl<'_>,
        mut leading: bool,
        mut advance: f64,
    ) -> Result<(bool, f64, BTreeMap<usize, f32>), LayoutComputationError> {
        let mut tab_units = BTreeMap::new();
        if config.basis() == super::super::WhitespaceBasis::ParagraphEn {
            let (leading, advance) = apply_whitespace_geometry(
                clusters,
                text,
                origin,
                paragraph_unit,
                config.tabstop,
                leading,
                advance,
            );
            return Ok((leading, advance, tab_units));
        }
        for (index, cluster) in clusters.iter_mut().enumerate() {
            if index % CANCELLATION_CLUSTER_BATCH == 0 {
                control.checkpoint()?;
            }
            let Some(spelling) =
                text.get(cluster.text_range.start - origin..cluster.text_range.end - origin)
            else {
                continue;
            };
            leading &= spelling.bytes().all(|b| b == b' ' || b == b'\t');
            if spelling.contains('\t') {
                let index = style_runs
                    .partition_point(|run| run.text_range.end <= cluster.text_range.start);
                let style = style_runs
                    .get(index)
                    .filter(|run| run.text_range.start <= cluster.text_range.start)
                    .map_or(default_style, |run| &run.style);
                let unit =
                    self.whitespace_unit(config, style, scale, document_id, revision, control)?;
                tab_units.insert(cluster.text_range.start, unit);
                (_, advance) = apply_whitespace_geometry(
                    std::slice::from_mut(cluster),
                    text,
                    origin,
                    unit,
                    config.tabstop,
                    false,
                    advance,
                );
            } else {
                advance += f64::from(cluster.advance);
            }
        }
        Ok((leading, advance, tab_units))
    }
}

pub(super) fn leading_indent_advance(clusters: &[ShapedCluster], text: &str, origin: usize) -> f64 {
    clusters
        .iter()
        .take_while(|cluster| {
            text[cluster.text_range.start - origin..cluster.text_range.end - origin]
                .bytes()
                .all(|byte| byte == b' ' || byte == b'\t')
        })
        .map(|cluster| f64::from(cluster.advance))
        .sum()
}

pub(super) fn attach_whitespace_units(row: &mut VisualRow, units: &BTreeMap<usize, f32>) {
    for cluster in &mut row.clusters {
        cluster.whitespace_unit = units.get(&cluster.text_range.start).copied();
    }
}

pub(super) fn apply_whitespace_geometry(
    clusters: &mut [ShapedCluster],
    text: &str,
    origin: usize,
    unit: f32,
    tabstop: u32,
    mut leading: bool,
    mut advance: f64,
) -> (bool, f64) {
    let interval = f64::from(unit) * f64::from(tabstop);
    for cluster in clusters {
        let Some(text) = cluster
            .text_range
            .start
            .checked_sub(origin)
            .and_then(|start| {
                cluster
                    .text_range
                    .end
                    .checked_sub(origin)
                    .and_then(|end| text.get(start..end))
            })
        else {
            continue;
        };
        let whitespace = text.bytes().all(|b| b == b' ' || b == b'\t');
        let adjusted = whitespace && (leading || text.contains('\t'));
        if adjusted {
            let old = cluster.advance;
            let start = advance;
            let mut boundaries = Vec::with_capacity(text.len() + 1);
            boundaries.push(0.0);
            for byte in text.bytes() {
                if byte == b'\t' {
                    advance = ((advance / interval + 1.0e-8).floor() + 1.0) * interval;
                } else if leading {
                    advance += f64::from(unit);
                } else {
                    // Ordinary body spaces keep shaping widths. Providers
                    // normally return individual ASCII whitespace clusters.
                    advance += f64::from(old) / text.len() as f64;
                }
                boundaries.push((advance - start) as f32);
            }
            cluster.advance = (advance - start) as f32;
            for stop in &mut cluster.caret_stops {
                if let Some(&x) = boundaries.get(stop.text_offset - cluster.text_range.start) {
                    stop.inline_offset = if cluster.bidi_level % 2 == 0 {
                        x
                    } else {
                        cluster.advance - x
                    };
                }
            }
            cluster.typographic_bounds.width = cluster.advance;
            cluster.ink_bounds = ShapedBounds::default();
            // Keep the blank native run as a font-resource reference for
            // marker style inheritance. Its glyphs have no visible ink.
        } else {
            advance += f64::from(cluster.advance);
        }
        leading &= whitespace;
    }
    (leading, advance)
}
