//! Randomized invariant checks for the native split-pane geometry model.
//!
//! The macOS wrapper (`EVPaneLayout.swift`) treats any non-OK status from
//! `update`, `drag`, `remove` and `copy` as a programming error and traps, so
//! these operations must succeed for every pane or splitter the layout itself
//! reports. After every operation the frames must tile the laid-out area
//! exactly, without overlap, and respect the minimum pane sizes.
//!
//! ```sh
//! cargo run --release --example fuzz_panes -- 20000 400
//! ```
use viem_core::layout::panes::{Axis, Error, Frame, PaneLayout, MIN_WIDTH, SPLITTER_WIDTH};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() % 1_000_000) as f64 / 1_000_000.0 * (hi - lo)
    }
}

const EPS: f64 = 0.01;

fn panes(layout: &PaneLayout) -> Vec<Frame> {
    layout.frames().into_iter().filter(|f| !f.splitter).collect()
}

fn check(layout: &PaneLayout, size: (f64, f64), chrome: &[(u64, f64)], label: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let frames = layout.frames();
    let (min_w, min_h) = layout.minimum();
    let area_w = size.0.max(min_w);
    let area_h = size.1.max(min_h);
    let mut covered = 0.0;
    for f in &frames {
        let r = f.rect;
        if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite()) {
            problems.push(format!("{label}: non-finite frame {f:?}"));
            continue;
        }
        if r.width < -EPS || r.height < -EPS {
            problems.push(format!("{label}: negative frame {f:?}"));
        }
        if r.x < -EPS || r.y < -EPS || r.x + r.width > area_w + EPS || r.y + r.height > area_h + EPS {
            problems.push(format!("{label}: frame outside {area_w}x{area_h}: {f:?}"));
        }
        if f.splitter {
            if (r.width - SPLITTER_WIDTH).abs() > EPS {
                problems.push(format!("{label}: splitter width {f:?}"));
            }
        } else {
            if r.width + EPS < MIN_WIDTH {
                problems.push(format!("{label}: pane narrower than {MIN_WIDTH}: {f:?}"));
            }
            let status = chrome.iter().find(|(id, _)| *id == f.id).map_or(0.0, |c| c.1);
            if r.height + EPS < status {
                problems.push(format!("{label}: pane shorter than its status bar {status}: {f:?}"));
            }
        }
        covered += r.width.max(0.0) * r.height.max(0.0);
    }
    for (i, a) in frames.iter().enumerate() {
        for b in &frames[i + 1..] {
            let w = (a.rect.x + a.rect.width).min(b.rect.x + b.rect.width) - a.rect.x.max(b.rect.x);
            let h = (a.rect.y + a.rect.height).min(b.rect.y + b.rect.height) - a.rect.y.max(b.rect.y);
            if w > EPS && h > EPS {
                problems.push(format!("{label}: overlap {a:?} / {b:?}"));
            }
        }
    }
    if (covered - area_w * area_h).abs() > 1.0 {
        problems.push(format!("{label}: frames cover {covered:.2} of {:.2}", area_w * area_h));
    }
    if panes(layout).len() != chrome.len() {
        problems.push(format!("{label}: {} panes but {} chrome entries", panes(layout).len(), chrome.len()));
    }
    problems
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cases: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let steps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let first: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures = std::collections::BTreeMap::<String, (usize, String)>::new();
    let mut stats = std::collections::BTreeMap::<String, usize>::new();
    let mut max_panes = 0;
    for case in first..first + cases {
        let mut rng = Rng(case.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let mut layout = PaneLayout::default();
        let status_height = *rng.pick(&[0.0, 22.0, 24.0, 30.5]);
        let mut chrome: Vec<(u64, f64)> = vec![(1, status_height)];
        let mut size = (rng.f(150.0, 1600.0), rng.f(60.0, 1200.0));
        let mut trace = Vec::new();
        let r = layout.update(size.0, size.1, &chrome);
        if r.is_err() {
            failures.entry("initial update failed".into()).or_insert((0, String::new())).0 += 1;
            continue;
        }
        for _ in 0..steps {
            let pane_ids: Vec<u64> = panes(&layout).iter().map(|f| f.id).collect();
            let splitters: Vec<u64> = layout.frames().iter().filter(|f| f.splitter).map(|f| f.id).collect();
            let pane = *rng.pick(&pane_ids);
            let op = rng.below(13);
            let label;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Option<String>, String> {
                Ok(match op {
                    0 | 1 => {
                        // Split: may fail with NoRoom/TooManyPanes, but can_split must agree.
                        let axis = if rng.below(2) == 0 { Axis::Width } else { Axis::Height };
                        let height = *rng.pick(&[0.0, 22.0, 24.0, 30.5]);
                        let can = layout.can_split(pane, axis, height);
                        match layout.split(pane, axis, height) {
                            Ok(id) => {
                                if can.is_err() {
                                    return Err(format!("split succeeded but can_split said {can:?}"));
                                }
                                chrome.push((id, height));
                                // The native wrapper immediately re-sends the current size and chrome.
                                layout.update(size.0, size.1, &chrome).map_err(|e| format!("update after split {e:?}"))?;
                                Some(format!("split {pane} {axis:?} {height} -> {id}"))
                            }
                            Err(e @ (Error::NoRoom | Error::TooManyPanes)) => {
                                if can.is_ok() {
                                    return Err(format!("split failed {e:?} but can_split said ok"));
                                }
                                Some(format!("split {pane} {axis:?} refused {e:?}"))
                            }
                            Err(e) => return Err(format!("split {pane} {axis:?}: {e:?}")),
                        }
                    }
                    2 if pane_ids.len() > 1 => {
                        layout.remove(pane).map_err(|e| format!("remove {pane}: {e:?}"))?;
                        chrome.retain(|(id, _)| *id != pane);
                        layout.update(size.0, size.1, &chrome).map_err(|e| format!("update after remove {e:?}"))?;
                        Some(format!("remove {pane}"))
                    }
                    3 | 4 => {
                        let delta = *rng.pick(&[-1000.0, -60.0, -7.5, -0.4, 0.3, 5.0, 33.0, 900.0]);
                        layout.drag(pane, true, delta).map_err(|e| format!("status drag {pane} {delta}: {e:?}"))?;
                        Some(format!("drag status {pane} {delta}"))
                    }
                    5 if !splitters.is_empty() => {
                        let id = *rng.pick(&splitters);
                        let delta = *rng.pick(&[-1000.0, -60.0, -7.5, -0.4, 0.3, 5.0, 33.0, 900.0]);
                        layout.drag(id, false, delta).map_err(|e| format!("splitter drag {id} {delta}: {e:?}"))?;
                        Some(format!("drag splitter {id} {delta}"))
                    }
                    6 => {
                        size = (rng.f(0.0, 2000.0), rng.f(0.0, 1400.0));
                        layout.update(size.0, size.1, &chrome).map_err(|e| format!("update {size:?}: {e:?}"))?;
                        Some(format!("resize window {size:?}"))
                    }
                    7 => {
                        // Status bar height change (font/theme).
                        let i = rng.below(chrome.len());
                        chrome[i].1 = *rng.pick(&[0.0, 18.0, 22.0, 40.0]);
                        layout.update(size.0, size.1, &chrome).map_err(|e| format!("update chrome: {e:?}"))?;
                        Some(format!("chrome {:?}", chrome[i]))
                    }
                    8 => {
                        layout.equalize(*rng.pick(&[None, Some(Axis::Height), Some(Axis::Width)]));
                        Some("equalize".into())
                    }
                    9 => {
                        let axis = if rng.below(2) == 0 { Axis::Width } else { Axis::Height };
                        let extent = *rng.pick(&[None, Some(0.0), Some(50.0), Some(-30.0), Some(400.0), Some(1e6)]);
                        let relative = rng.below(2) == 0;
                        layout.resize(pane, axis, extent, relative).map_err(|e| format!("resize {pane} {axis:?} {extent:?} {relative}: {e:?}"))?;
                        Some(format!("resize {pane} {axis:?} {extent:?} rel {relative}"))
                    }
                    10 => {
                        let direction = rng.below(4) as u32;
                        let id = layout.focus(pane, direction, 1 + rng.below(3), (rng.f(0.0, 2000.0), rng.f(0.0, 2000.0)))
                            .map_err(|e| format!("focus: {e:?}"))?;
                        if !pane_ids.contains(&id) {
                            return Err(format!("focus returned non-pane {id}"));
                        }
                        Some(format!("focus {pane} {direction} -> {id}"))
                    }
                    11 => match if rng.below(2) == 0 { layout.rotate(pane, 1 + rng.below(3) as u64, rng.below(2) == 0) } else { layout.reorder(pane, None, None) } {
                        Ok(()) | Err(Error::Unsupported) => Some(format!("rotate/exchange {pane}")),
                        Err(e) => return Err(format!("rotate/exchange {pane}: {e:?}")),
                    },
                    12 => match layout.move_edge(pane, rng.below(4) as u32) {
                        Ok(()) | Err(Error::NoRoom) => {
                            layout.update(size.0, size.1, &chrome).map_err(|e| format!("update after move: {e:?}"))?;
                            Some(format!("move edge {pane}"))
                        }
                        Err(e) => return Err(format!("move_edge {pane}: {e:?}")),
                    },
                    _ => None,
                })
            }));
            match result {
                Err(_) => {
                    let key = format!("PANIC op {op}");
                    let entry = failures.entry(key).or_insert((0, String::new()));
                    entry.0 += 1;
                    if entry.1.is_empty() {
                        entry.1 = format!("case {case} trace {trace:?}");
                    }
                    break;
                }
                Ok(Err(message)) => {
                    let key = message.split(':').next().unwrap_or("").chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
                    let entry = failures.entry(format!("error {key}")).or_insert((0, String::new()));
                    entry.0 += 1;
                    if entry.1.is_empty() || entry.1.len() > trace.len() * 60 + 200 {
                        entry.1 = format!("{message} | case {case} size {size:?} trace {trace:?}");
                    }
                    break;
                }
                Ok(Ok(Some(text))) => {
                    max_panes = max_panes.max(panes(&layout).len());
                    *stats.entry(text.split_whitespace().take(2).collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_ascii_digit()).collect()).or_default() += 1;
                    label = text;
                    trace.push(label.clone());
                    let problems = check(&layout, size, &chrome, &label);
                    if let Some(first) = problems.first() {
                        let key = first.split(':').nth(1).unwrap_or("").split_whitespace().take(3).collect::<Vec<_>>().join(" ");
                        let entry = failures.entry(format!("invariant {key}")).or_insert((0, String::new()));
                        entry.0 += 1;
                        if entry.1.is_empty() || entry.1.len() > trace.len() * 60 + 200 {
                            entry.1 = format!("{first} | case {case} size {size:?} chrome {chrome:?} trace {trace:?}");
                        }
                        break;
                    }
                }
                Ok(Ok(None)) => {}
            }
        }
    }
    for (key, (count, example)) in &failures {
        println!("{count:6} {key}\n       {}", example.chars().take(1500).collect::<String>());
    }
    println!("stats {stats:?} max panes {max_panes}");
    println!("{} failure kinds over {cases} cases", failures.len());
}
