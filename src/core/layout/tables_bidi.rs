//! Compact bidi context for a materialized logical band. Under UAX #9 L2, two
//! clusters exchange order exactly when the minimum level between them is odd.
//! Width buckets retain that minimum without retaining offscreen glyphs/runs.
#[derive(Clone, Debug)]
pub(super) struct BidiBandWidths {
    prefix: [f64; 256],
    prefix_high: u8,
    suffix: [f64; 256],
    suffix_min: u8,
}

impl Default for BidiBandWidths {
    fn default() -> Self {
        Self {
            prefix: [0.; 256],
            prefix_high: 0,
            suffix: [0.; 256],
            suffix_min: u8::MAX,
        }
    }
}

impl BidiBandWidths {
    /// Append a cluster before the band in logical order. Each bucket names
    /// the minimum level from its contributors to the current prefix end.
    pub(super) fn push_prefix(&mut self, level: u8, width: f32) {
        let index = usize::from(level);
        let collapsed = if level <= self.prefix_high {
            let active = &mut self.prefix[index..=usize::from(self.prefix_high)];
            let width = active.iter().sum::<f64>();
            active.fill(0.);
            width
        } else {
            0.
        };
        self.prefix[index] = collapsed + f64::from(width);
        // No bucket above the newest level can remain after the collapse.
        // Uniform text consequently touches one bucket per glyph, not 256.
        self.prefix_high = level;
    }

    /// Append a cluster after the band in logical order. Its relevant minimum
    /// extends from the band end through all earlier suffix clusters.
    pub(super) fn push_suffix(&mut self, level: u8, width: f32) {
        self.suffix_min = self.suffix_min.min(level);
        self.suffix[usize::from(self.suffix_min)] += f64::from(width);
    }

    pub(super) fn total_width(&self, band_width: f64) -> f32 {
        (self.prefix.iter().sum::<f64>() + band_width + self.suffix.iter().sum::<f64>()) as f32
    }

    /// Return visual x for each band cluster, supplied in logical order.
    /// Glyph advances stay untouched; the returned offsets include all text
    /// before and after the band that bidi places to the cluster's left.
    pub(super) fn positions(&self, levels: &[u8], widths: &[f32]) -> Vec<f32> {
        assert_eq!(levels.len(), widths.len());
        if levels.is_empty() {
            return Vec::new();
        }
        let prefix_before = contributions(&self.prefix, false);
        let suffix_before = contributions(&self.suffix, true);
        let mut positions = vec![0.; levels.len()];
        let mut suffix_min = u8::MAX;
        for (index, &level) in levels.iter().enumerate().rev() {
            suffix_min = suffix_min.min(level);
            positions[index] = suffix_before[usize::from(suffix_min)];
        }
        let mut prefix_min = u8::MAX;
        for (index, &level) in levels.iter().enumerate() {
            prefix_min = prefix_min.min(level);
            positions[index] += prefix_before[usize::from(prefix_min)];
        }
        let mut order = (0..levels.len()).collect::<Vec<_>>();
        if let Some(lowest) = levels.iter().copied().filter(|level| level % 2 == 1).min() {
            let highest = levels.iter().copied().max().unwrap_or(lowest);
            for level in (lowest..=highest).rev() {
                let mut index = 0;
                while index < order.len() {
                    while index < order.len() && levels[order[index]] < level {
                        index += 1;
                    }
                    let first = index;
                    while index < order.len() && levels[order[index]] >= level {
                        index += 1;
                    }
                    order[first..index].reverse();
                }
            }
        }
        let mut x = 0.;
        for index in order {
            positions[index] += x;
            x += f64::from(widths[index]);
        }
        positions.into_iter().map(|x| x as f32).collect()
    }
}

/// For an in-band minimum m, an external bucket contributes when min(level,m)
/// has the requested parity. One pass builds all possible m lookups.
fn contributions(buckets: &[f64; 256], odd: bool) -> [f64; 256] {
    let mut result = [0.; 256];
    let mut remaining = buckets.iter().sum::<f64>();
    let mut lower = 0.;
    for (level, &width) in buckets.iter().enumerate() {
        let matches = (level % 2 == 1) == odd;
        result[level] = lower + if matches { remaining } else { 0. };
        if matches {
            lower += width;
        }
        remaining -= width;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent pairwise L2 oracle: every cluster contributes to x exactly
    /// when it precedes the target after the intervening reversals.
    fn reference(levels: &[u8], widths: &[f32], target: usize) -> f32 {
        (0..levels.len())
            .filter(|&other| {
                other != target && {
                    let low = other.min(target);
                    let high = other.max(target);
                    let reversed = levels[low..=high].iter().min().unwrap() % 2 == 1;
                    (other < target) != reversed
                }
            })
            .map(|index| widths[index])
            .sum()
    }

    #[test]
    fn sparse_bands_match_full_l2_order_including_nested_runs() {
        let mut seed = 7u32;
        let cases = [
            vec![0; 160],
            vec![1; 160],
            (0..160)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    ((seed >> 16) % 8) as u8
                })
                .collect(),
            (0..160)
                .map(|at| {
                    if at < 12 || at > 120 {
                        0
                    } else if at < 90 {
                        1
                    } else {
                        2
                    }
                })
                .collect(),
            vec![126; 160],
        ];
        let widths = (0..160)
            .map(|at| 1. + (at % 5) as f32 / 4.)
            .collect::<Vec<_>>();
        for levels in cases {
            for band in [0..20, 20..50, 50..110, 140..160, 0..160] {
                let mut context = BidiBandWidths::default();
                for at in 0..band.start {
                    context.push_prefix(levels[at], widths[at]);
                }
                for at in band.end..levels.len() {
                    context.push_suffix(levels[at], widths[at]);
                }
                let positions = context.positions(&levels[band.clone()], &widths[band.clone()]);
                for (index, actual) in positions.into_iter().enumerate() {
                    assert_eq!(
                        actual,
                        reference(&levels, &widths, band.start + index),
                        "band {band:?}, cluster {index}, levels {levels:?}"
                    );
                }
            }
        }
    }
}
