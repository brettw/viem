//! Word 97+ list-table interpretation. Table bytes and cached list labels stay
//! in the lossless token stream; only supported decimal/bullet levels project.
use super::rtf::{Kind, Token};
use super::BlockProperties;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) struct Level {
    pub ordered: bool,
    pub start: u64,
    pub no_restart: bool,
    pub paragraph: BlockProperties,
}
#[derive(Default)]
pub(super) struct ListTables {
    levels: BTreeMap<i32, Vec<Option<Level>>>,
}
#[derive(Default)]
struct Group {
    destination: String,
    direct: Vec<usize>,
    children: Vec<usize>,
    parent: Option<usize>,
}
impl Group {
    fn number(&self, tokens: &[Token], name: &str) -> Option<i32> {
        self.direct
            .iter()
            .rev()
            .find_map(|&index| match &tokens[index].kind {
                Kind::Control(control, value) if control == name => *value,
                _ => None,
            })
    }
    fn has(&self, tokens: &[Token], name: &str) -> bool {
        self.direct.iter().any(|&index| {
            matches!(&tokens[index].kind,
            Kind::Control(control, _) if control == name)
        })
    }
}
impl ListTables {
    pub fn read(tokens: &[Token]) -> Self {
        if !tokens
            .iter()
            .any(|token| matches!(&token.kind,Kind::Control(name,_) if name=="listtable"))
        {
            return Self::default();
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            match &token.kind {
                Kind::Open => {
                    let parent = stack.last().copied();
                    let id = groups.len();
                    groups.push(Group {
                        parent,
                        ..Group::default()
                    });
                    if let Some(parent) = parent {
                        groups[parent].children.push(id);
                    }
                    stack.push(id);
                }
                Kind::Close => {
                    stack.pop();
                }
                _ => {
                    if let Some(&id) = stack.last() {
                        if let Kind::Control(name, _) = &token.kind {
                            if groups[id].destination.is_empty() {
                                groups[id].destination = name.clone();
                            }
                        }
                        groups[id].direct.push(index);
                    }
                }
            }
        }
        let mut definitions: BTreeMap<i32, Vec<Option<Level>>> = BTreeMap::new();
        for group in groups.iter().filter(|g| {
            g.destination == "list"
                && g.parent.is_some_and(|p| {
                    groups[p].destination == "listtable"
                        && groups[p].parent.is_some_and(|root| {
                            groups[root].parent.is_none() && groups[root].destination == "rtf"
                        })
                })
        }) {
            let Some(id) = group.number(tokens, "listid") else {
                continue;
            };
            let levels = group
                .children
                .iter()
                .filter(|&&i| groups[i].destination == "listlevel")
                .take(9)
                .enumerate()
                .map(|(level, &i)| read_level(&groups[i], level as u8, &groups, tokens))
                .collect();
            definitions.entry(id).or_insert(levels);
        }
        let mut result = Self::default();
        for group in groups.iter().filter(|g| {
            g.destination == "listoverride"
                && g.parent.is_some_and(|p| {
                    groups[p].destination == "listoverridetable"
                        && groups[p].parent.is_some_and(|root| {
                            groups[root].parent.is_none() && groups[root].destination == "rtf"
                        })
                })
        }) {
            let Some(id) = group
                .number(tokens, "ls")
                .filter(|n| (1..=2000).contains(n))
            else {
                continue;
            };
            let Some(mut levels) = group
                .number(tokens, "listid")
                .and_then(|id| definitions.get(&id))
                .cloned()
            else {
                continue;
            };
            for (level, &child) in group
                .children
                .iter()
                .filter(|&&i| groups[i].destination == "lfolevel")
                .take(9)
                .enumerate()
            {
                let override_group = &groups[child];
                if level >= levels.len() {
                    break;
                }
                if override_group
                    .number(tokens, "listoverrideformat")
                    .is_some_and(|n| n > 0)
                {
                    levels[level] = override_group
                        .children
                        .iter()
                        .find(|&&i| groups[i].destination == "listlevel")
                        .and_then(|&i| read_level(&groups[i], level as u8, &groups, tokens));
                }
                if override_group.has(tokens, "listoverridestartat") {
                    let start = override_group.number(tokens, "levelstartat").or_else(|| {
                        override_group
                            .children
                            .iter()
                            .find_map(|&i| groups[i].number(tokens, "levelstartat"))
                    });
                    if let Some(start) = start.and_then(|n| u64::try_from(n).ok()) {
                        if let Some(value) = &mut levels[level] {
                            value.start = start;
                        }
                    }
                }
            }
            result.levels.entry(id).or_insert(levels);
        }
        result
    }
    pub fn level(&self, id: i32, level: u8) -> Option<&Level> {
        self.levels.get(&id)?.get(usize::from(level))?.as_ref()
    }
}
fn read_level(group: &Group, level: u8, groups: &[Group], tokens: &[Token]) -> Option<Level> {
    let nfc = group
        .number(tokens, "levelnfcn")
        .or_else(|| group.number(tokens, "levelnfc"))?;
    let ordered = match nfc {
        0 => true,
        23 => false,
        _ => return None,
    };
    if group.has(tokens, "levelpicture") {
        return None;
    }
    if ordered {
        let text = group
            .children
            .iter()
            .find(|&&i| groups[i].destination == "leveltext")?;
        let text: Vec<u8> = groups[*text]
            .direct
            .iter()
            .filter_map(|&i| match tokens[i].kind {
                Kind::Byte(value) => Some(value),
                Kind::Character(c) if c.is_ascii() && !matches!(c, '\r' | '\n') => Some(c as u8),
                _ => None,
            })
            .collect();
        // The supported decimal policy is the current level followed by '.';
        // compound or differently punctuated patterns remain opaque.
        if text.as_slice() != [2, level, b'.', b';'] && text.as_slice() != [2, level, b'.'] {
            return None;
        }
    }
    Some(Level {
        ordered,
        start: group
            .number(tokens, "levelstartat")
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(1),
        no_restart: group.number(tokens, "levelnorestart") == Some(1),
        paragraph: BlockProperties {
            leading_indent: group.number(tokens, "li").map(|n| n as f32 / 20.0),
            first_line_indent: group.number(tokens, "fi").map(|n| n as f32 / 20.0),
            ..BlockProperties::default()
        },
    })
}

/// Numbering runs belong to the document stream, rather than the RTF group
/// stack: leaving a character-format group never rewinds a list counter.
#[derive(Default)]
pub(super) struct Numbering {
    counters: BTreeMap<(i32, u8), u64>,
    containers: [Option<i32>; 9],
}
impl Numbering {
    pub fn next(&mut self, tables: &ListTables, id: i32, level: u8) -> Option<(bool, u64, bool)> {
        let definition = tables.level(id, level)?;
        let ordinal = self
            .counters
            .entry((id, level))
            .and_modify(|n| *n = n.saturating_add(1))
            .or_insert(definition.start);
        let ordinal = *ordinal;
        for child in level + 1..9 {
            if tables.level(id, child).is_some_and(|l| !l.no_restart) {
                self.counters.remove(&(id, child));
            }
        }
        let first = self.containers[usize::from(level)] != Some(id);
        self.containers[usize::from(level)] = Some(id);
        for child in usize::from(level) + 1..9 {
            self.containers[child] = None;
        }
        Some((definition.ordered, ordinal, first))
    }
    pub fn leave(&mut self) {
        self.containers = [None; 9];
    }
}
