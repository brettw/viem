//! Portable advances for whitespace, applied after shaping and before wrapping.
use super::*;

impl<P: TextMeasurementProvider> LayoutEngine<P> {
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
