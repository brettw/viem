//! Preserve structural owners when an explicit format conversion serializes
//! paragraph leaves. Source correspondence remains owned by ConversionWriter.
use super::*;
use crate::document::{Block, ContainerIdentity, ContainerKind, ContainerMembership};

fn same_prefix(left: &[ContainerMembership], right: &[ContainerMembership]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(a, b)| a.container.id == b.container.id)
        .count()
}
fn item(path: &[ContainerMembership]) -> Option<ContainerIdentity> {
    path.iter()
        .rev()
        .find(|m| m.container.kind == ContainerKind::ListItem)
        .map(|m| m.container.id)
}
fn heading(block: &Block) -> Option<u8> {
    if let BlockKind::Heading(level) = block.kind {
        Some(level)
    } else {
        block
            .style
            .0
            .strip_prefix("Heading")
            .and_then(|s| s.parse().ok())
            .filter(|n| (1..=6).contains(n))
    }
}
fn numbering(blocks: &[Block]) -> BTreeMap<ContainerIdentity, (bool, u64)> {
    let mut numbers = BTreeMap::new();
    let mut next = BTreeMap::new();
    for block in blocks {
        let mut owner = None;
        for member in block.containers.iter() {
            match member.container.kind {
                ContainerKind::List { ordered } => {
                    owner = Some((member.container.id, ordered));
                }
                ContainerKind::ListItem => {
                    let Some((list, ordered)) = owner else {
                        continue;
                    };
                    if numbers.contains_key(&member.container.id) {
                        continue;
                    }
                    let counter = next.entry(list).or_insert(1);
                    let ordinal = if item(&block.containers) == Some(member.container.id) {
                        if let BlockKind::ListItem { ordinal, .. } = block.kind {
                            u64::from(ordinal)
                        } else {
                            *counter
                        }
                    } else {
                        *counter
                    };
                    numbers.insert(member.container.id, (ordered, ordinal));
                    *counter = ordinal.saturating_add(1);
                }
                _ => {}
            }
        }
    }
    numbers
}

pub(super) fn write(
    document: &FormattedDocument,
    blocks: &[Block],
    target: Format,
    losses: &mut BTreeSet<ConversionLoss>,
    output: &mut ConversionWriter,
) {
    let numbers = numbering(blocks);
    if target == Format::Html {
        html(document, blocks, &numbers, losses, output);
    } else {
        markdown(document, blocks, &numbers, losses, output);
    }
}

fn html(
    document: &FormattedDocument,
    blocks: &[Block],
    numbers: &BTreeMap<ContainerIdentity, (bool, u64)>,
    losses: &mut BTreeSet<ConversionLoss>,
    output: &mut ConversionWriter,
) {
    let mut paragraphs = BTreeMap::<_, usize>::new();
    for block in blocks {
        if let Some(owner) = item(&block.containers) {
            *paragraphs.entry(owner).or_default() += 1;
        }
    }
    let close = |path: &[ContainerMembership], output: &mut ConversionWriter| {
        for member in path.iter().rev() {
            output.push_str(match member.container.kind {
                ContainerKind::Quote => "</blockquote>",
                ContainerKind::CodeBlock => "</code></pre>",
                ContainerKind::List { ordered: true } => "</ol>",
                ContainerKind::List { ordered: false } => "</ul>",
                ContainerKind::ListItem => "</li>",
            });
        }
    };
    let mut previous: &[ContainerMembership] = &[];
    for (index, block) in blocks.iter().enumerate() {
        let path = block.containers.as_ref();
        let common = same_prefix(previous, path);
        close(&previous[common..], output);
        if index > 0 && common == 0 {
            output.push('\n');
        }
        for (depth, member) in path.iter().enumerate().skip(common) {
            match member.container.kind {
                ContainerKind::Quote => output.push_str("<blockquote>"),
                ContainerKind::CodeBlock => output.push_str("<pre><code>"),
                ContainerKind::List { ordered } => {
                    if ordered {
                        let ordinal = path[depth + 1..]
                            .iter()
                            .find_map(|m| numbers.get(&m.container.id).map(|n| n.1))
                            .unwrap_or(1);
                        output.push_str(&format!("<ol start=\"{ordinal}\">"));
                    } else {
                        output.push_str("<ul>");
                    }
                }
                ContainerKind::ListItem => {
                    let (ordered, ordinal) = numbers
                        .get(&member.container.id)
                        .copied()
                        .unwrap_or((false, 1));
                    let value = if ordered {
                        format!(" value=\"{ordinal}\"")
                    } else {
                        String::new()
                    };
                    output.push_str(&format!(
                        "<li{value}{}>",
                        whitespace_attribute(&document.text()[block.range.clone()])
                    ));
                }
            }
        }
        let code = path
            .iter()
            .any(|m| m.container.kind == ContainerKind::CodeBlock);
        if code {
            output.text(
                &document.text()[block.range.clone()],
                block.range.start,
                TextSpelling::Html { hard_breaks: false },
            );
        } else {
            let direct_item = path
                .last()
                .is_some_and(|m| m.container.kind == ContainerKind::ListItem)
                && item(path).is_some_and(|id| paragraphs.get(&id) == Some(&1));
            let tag = heading(block)
                .map(|n| format!("h{n}"))
                .or_else(|| (!direct_item).then(|| "p".to_owned()));
            if let Some(tag) = &tag {
                output.push_str(&format!(
                    "<{tag}{}>",
                    whitespace_attribute(&document.text()[block.range.clone()])
                ));
            }
            inline(document, block.range.clone(), Format::Html, losses, output);
            if let Some(tag) = &tag {
                output.push_str(&format!("</{tag}>"));
            }
        }
        previous = path;
    }
    close(previous, output);
}

fn prefix(
    path: &[ContainerMembership],
    numbers: &BTreeMap<ContainerIdentity, (bool, u64)>,
    first: bool,
) -> String {
    let mut value = String::new();
    for member in path {
        match member.container.kind {
            ContainerKind::Quote => value.push_str("> "),
            ContainerKind::ListItem => {
                let (ordered, ordinal) = numbers
                    .get(&member.container.id)
                    .copied()
                    .unwrap_or((false, 1));
                let marker = if ordered {
                    format!("{ordinal}. ")
                } else {
                    "- ".to_owned()
                };
                if first && member.starts_here {
                    value.push_str(&marker);
                } else {
                    value.push_str(&" ".repeat(marker.len()));
                }
            }
            _ => {}
        }
    }
    value
}
fn markdown(
    document: &FormattedDocument,
    blocks: &[Block],
    numbers: &BTreeMap<ContainerIdentity, (bool, u64)>,
    losses: &mut BTreeSet<ConversionLoss>,
    output: &mut ConversionWriter,
) {
    for (index, block) in blocks.iter().enumerate() {
        let path = block.containers.as_ref();
        if index > 0 {
            let previous = &blocks[index - 1];
            let common = same_prefix(&previous.containers, path);
            output.push('\n');
            let new_item = item(path) != item(&previous.containers) && item(path).is_some();
            // An ordered list starting other than one cannot interrupt prose in GFM.
            // Keep its boundary explicit, including inside quotes/list items.
            let numbered_start_needs_break = path.iter().any(|member| {
                member.starts_here && member.container.kind == ContainerKind::List { ordered: true }
            }) && item(path)
                .and_then(|id| numbers.get(&id))
                .is_some_and(|(_, ordinal)| *ordinal != 1);
            let separates_quotes = previous.containers[common..]
                .iter()
                .any(|m| m.container.kind == ContainerKind::Quote);
            let separate = separates_quotes
                || (!new_item && (heading(previous).is_none() || item(path).is_some()))
                || (new_item && (block.list_loose || numbered_start_needs_break));
            if separate {
                output.push_str(&prefix(&path[..common], numbers, false));
                output.push('\n');
            }
        }
        if block.direct_paragraph != Default::default()
            || block.direct_default_character != Default::default()
            || path.iter().any(|m| m.container.direct_formatting.is_some())
        {
            losses.insert(ConversionLoss::Styling);
        }
        let opening = prefix(path, numbers, true);
        let continuation = prefix(path, numbers, false);
        output.push_str(&opening);
        output.line_prefix = MarkdownLinePrefix::Empty;
        if path
            .iter()
            .any(|m| m.container.kind == ContainerKind::CodeBlock)
            || block.style.0 == "Code Block"
        {
            let text = &document.text()[block.range.clone()];
            let fence = "`"
                .repeat(longest_run(&output.visible_text(text, block.range.start), '`').max(2) + 1);
            output.push_str(&fence);
            output.push('\n');
            output.push_str(&continuation);
            let mut start = block.range.start;
            for fragment in text.split_inclusive('\n') {
                output.text(fragment, start, TextSpelling::Literal);
                start += fragment.len();
                if fragment.ends_with('\n') {
                    output.push_str(&continuation);
                }
            }
            output.push('\n');
            output.push_str(&continuation);
            output.push_str(&fence);
        } else {
            if let Some(level) = heading(block) {
                output.push_str(&format!("{} ", "#".repeat(usize::from(level))));
            }
            output.line_prefix = MarkdownLinePrefix::Empty;
            inline(
                document,
                block.range.clone(),
                Format::Markdown,
                losses,
                output,
            );
        }
    }
}
