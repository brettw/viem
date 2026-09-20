//! WHATWG tree construction with source-bound text. The DOM is disposable;
//! original bytes and lossless syntax remain the persistence authority.
use super::html::{Tag, Token, TokenKind};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{self, BufferQueue, TokenSink, TokenSinkResult, Tokenizer};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeSink};
use html5ever::{Attribute, ExpandedName, QualName};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::{Rc, Weak};

#[derive(Clone, Debug)]
struct Unit {
    value: char,
    source: Option<Range<usize>>,
}
type Handle = Rc<Node>;
struct Node {
    parent: RefCell<Weak<Node>>,
    children: RefCell<Vec<Handle>>,
    data: Data,
    source: Option<Range<usize>>,
    end: RefCell<Option<Range<usize>>>,
}
enum Data {
    Document,
    Element {
        name: QualName,
        attrs: RefCell<Vec<Attribute>>,
        template: Option<Handle>,
        integration: bool,
    },
    Text(RefCell<Vec<Unit>>),
    Ignored,
}
impl Node {
    fn new(data: Data, source: Option<Range<usize>>) -> Handle {
        Rc::new(Self {
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
            data,
            source,
            end: RefCell::new(None),
        })
    }
}
#[derive(Default)]
struct Context {
    current: VecDeque<Unit>,
    buffered: VecDeque<Unit>,
    tag: Option<(tokenizer::Tag, Range<usize>)>,
}
impl Context {
    fn take_text(&mut self, text: &str) -> Vec<Unit> {
        let values = text.chars().collect::<Vec<_>>();
        if self
            .current
            .iter()
            .take(values.len())
            .map(|unit| unit.value)
            .eq(values.iter().copied())
            && self.current.len() >= values.len()
        {
            return self.current.drain(..values.len()).collect();
        }
        let candidates = (0..=self.buffered.len().saturating_sub(values.len()))
            .filter(|start| {
                self.buffered.len() >= values.len()
                    && self
                        .buffered
                        .iter()
                        .skip(*start)
                        .take(values.len())
                        .map(|unit| unit.value)
                        .eq(values.iter().copied())
            })
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            let start = candidates[0];
            return self.buffered.drain(start..start + values.len()).collect();
        }
        // Ignored and foster-parented character buffers may have identical
        // spelling. Keep the semantic text but expose no invented provenance.
        if let Some(start) = candidates.first().copied() {
            self.buffered.drain(start..start + values.len());
        }
        values
            .into_iter()
            .map(|value| Unit {
                value,
                source: None,
            })
            .collect()
    }
}
struct Sink {
    document: Handle,
    context: Rc<RefCell<Context>>,
    elements: RefCell<Vec<Handle>>,
}
impl Sink {
    fn detach(node: &Handle) {
        if let Some(parent) = std::mem::take(&mut *node.parent.borrow_mut()).upgrade() {
            parent
                .children
                .borrow_mut()
                .retain(|child| !Rc::ptr_eq(child, node));
        }
    }
    fn insert(&self, parent: &Handle, index: usize, child: NodeOrText<Handle>) {
        match child {
            NodeOrText::AppendNode(node) => {
                Self::detach(&node);
                *node.parent.borrow_mut() = Rc::downgrade(parent);
                let mut children = parent.children.borrow_mut();
                let index = index.min(children.len());
                children.insert(index, node);
            }
            NodeOrText::AppendText(text) => {
                let units = self.context.borrow_mut().take_text(&text);
                let previous = parent
                    .children
                    .borrow()
                    .get(index.saturating_sub(1))
                    .filter(|_| index > 0)
                    .cloned();
                if let Some(previous) = previous {
                    if let Data::Text(value) = &previous.data {
                        value.borrow_mut().extend(units);
                        return;
                    }
                }
                let node = Node::new(Data::Text(RefCell::new(units)), None);
                *node.parent.borrow_mut() = Rc::downgrade(parent);
                let mut children = parent.children.borrow_mut();
                let index = index.min(children.len());
                children.insert(index, node);
            }
        }
    }
}
impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Self;
    type ElemName<'a> = ExpandedName<'a>;
    fn finish(self) -> Self {
        self
    }
    fn parse_error(&self, _: Cow<'static, str>) {}
    fn get_document(&self) -> Handle {
        self.document.clone()
    }
    fn elem_name<'a>(&'a self, node: &'a Handle) -> ExpandedName<'a> {
        match &node.data {
            Data::Element { name, .. } => name.expanded(),
            _ => unreachable!(),
        }
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let context = self.context.borrow();
        let source=context.tag.as_ref().filter(|(tag,_)|tag.kind==tokenizer::StartTag&&tag.name==name.local).map(|(_,range)|range.clone()).or_else(|| {
            // Adoption-agency clones inherit their original opening token.
            let candidates=self.elements.borrow().iter().filter(|node|matches!(&node.data,Data::Element{name:old,attrs:old_attrs,..}if old==&name&&*old_attrs.borrow()==attrs)).filter_map(|node|node.source.clone()).collect::<Vec<_>>();
            candidates.first().filter(|first|candidates.iter().all(|range|range==*first)).cloned()
        });
        let node = Node::new(
            Data::Element {
                name,
                attrs: RefCell::new(attrs),
                template: flags.template.then(|| Node::new(Data::Document, None)),
                integration: flags.mathml_annotation_xml_integration_point,
            },
            source,
        );
        self.elements.borrow_mut().push(node.clone());
        node
    }
    fn create_comment(&self, _: StrTendril) -> Handle {
        Node::new(Data::Ignored, None)
    }
    fn create_pi(&self, _: StrTendril, _: StrTendril) -> Handle {
        Node::new(Data::Ignored, None)
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        let length = parent.children.borrow().len();
        self.insert(parent, length, child);
    }
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        if let Some(parent) = sibling.parent.borrow().upgrade() {
            if let NodeOrText::AppendNode(node) = &child {
                Self::detach(node);
            }
            let index = parent
                .children
                .borrow()
                .iter()
                .position(|node| Rc::ptr_eq(node, sibling))
                .unwrap();
            self.insert(&parent, index, child);
        }
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        previous: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if element.parent.borrow().upgrade().is_some() {
            self.append_before_sibling(element, child)
        } else {
            self.append(previous, child)
        }
    }
    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}
    fn get_template_contents(&self, node: &Handle) -> Handle {
        match &node.data {
            Data::Element {
                template: Some(contents),
                ..
            } => contents.clone(),
            _ => unreachable!(),
        }
    }
    fn same_node(&self, a: &Handle, b: &Handle) -> bool {
        Rc::ptr_eq(a, b)
    }
    fn set_quirks_mode(&self, _: QuirksMode) {}
    fn add_attrs_if_missing(&self, node: &Handle, attrs: Vec<Attribute>) {
        if let Data::Element { attrs: old, .. } = &node.data {
            let mut old = old.borrow_mut();
            for attr in attrs {
                if !old.iter().any(|item| item.name == attr.name) {
                    old.push(attr)
                }
            }
        }
    }
    fn remove_from_parent(&self, node: &Handle) {
        Self::detach(node)
    }
    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        for child in std::mem::take(&mut *node.children.borrow_mut()) {
            *child.parent.borrow_mut() = Rc::downgrade(new_parent);
            new_parent.children.borrow_mut().push(child)
        }
    }
    fn is_mathml_annotation_xml_integration_point(&self, node: &Handle) -> bool {
        matches!(
            &node.data,
            Data::Element {
                integration: true,
                ..
            }
        )
    }
    fn pop(&self, node: &Handle) {
        let context = self.context.borrow();
        if let Some((tag, range)) = &context.tag {
            if tag.kind == tokenizer::EndTag
                && matches!(&node.data,Data::Element{name,..}if name.local==tag.name)
            {
                *node.end.borrow_mut() = Some(range.clone());
            }
        }
    }
}

struct Tracking<'a> {
    tree: TreeBuilder<Handle, Sink>,
    queue: Rc<BufferQueue>,
    input: &'a str,
    cursor: Cell<usize>,
    entity: RefCell<VecDeque<Unit>>,
    decode_entities: Cell<bool>,
}
impl Tracking<'_> {
    fn consumed(&self) -> usize {
        let copy = (*self.queue).clone();
        let mut remaining = 0;
        while let Some(chunk) = copy.pop_front() {
            remaining += chunk.len();
        }
        self.input.len().saturating_sub(remaining)
    }
    fn characters(&self, text: &str) -> VecDeque<Unit> {
        let mut result = VecDeque::new();
        for value in text.chars() {
            if let Some(unit) = self.entity.borrow_mut().pop_front() {
                if unit.value == value {
                    result.push_back(unit);
                    continue;
                }
            }
            let at = self.cursor.get();
            let tail = &self.input[at..];
            if self.decode_entities.get() && tail.starts_with('&') {
                if let Some((decoded, length)) = super::html::reference(tail, false) {
                    if decoded.starts_with(value) {
                        let source = Some(at..at + length);
                        let mut chars = decoded.chars();
                        chars.next();
                        self.entity.borrow_mut().extend(chars.map(|value| Unit {
                            value,
                            source: source.clone(),
                        }));
                        self.cursor.set(at + length);
                        result.push_back(Unit { value, source });
                        continue;
                    }
                }
            }
            if let Some(raw) = tail.chars().next() {
                if raw == value
                    || (raw == '\r' && value == '\n')
                    || (raw == '\0' && value == '\u{fffd}')
                {
                    let length = if raw == '\r' && tail.starts_with("\r\n") {
                        2
                    } else {
                        raw.len_utf8()
                    };
                    self.cursor.set(at + length);
                    result.push_back(Unit {
                        value,
                        source: Some(at..at + length),
                    });
                    continue;
                }
            }
            result.push_back(Unit {
                value,
                source: None,
            });
            self.cursor.set(self.consumed());
        }
        result
    }
}
impl TokenSink for Tracking<'_> {
    type Handle = Handle;
    fn process_token(&self, token: tokenizer::Token, line: u64) -> TokenSinkResult<Handle> {
        let context = &self.tree.sink.context;
        if matches!(token, tokenizer::ParseError(_)) {
            return self.tree.process_token(token, line);
        }
        context.borrow_mut().tag = None;
        match &token {
            tokenizer::CharacterTokens(text) => {
                context.borrow_mut().current = self.characters(text)
            }
            tokenizer::NullCharacterToken => context.borrow_mut().current = self.characters("\0"),
            tokenizer::TagToken(tag) => {
                let end = self.consumed();
                let start = self.cursor.replace(end);
                context.borrow_mut().tag = Some((tag.clone(), start..end));
                if tag.kind == tokenizer::EndTag {
                    self.decode_entities.set(true);
                }
            }
            tokenizer::CommentToken(_) | tokenizer::DoctypeToken(_) => {
                self.cursor.set(self.consumed());
            }
            _ => {}
        }
        let result = self.tree.process_token(token, line);
        {
            let mut context = context.borrow_mut();
            let remaining = std::mem::take(&mut context.current);
            context.buffered.extend(remaining);
        }
        match &result {
            TokenSinkResult::RawData(kind) => self
                .decode_entities
                .set(matches!(kind, tokenizer::states::RawKind::Rcdata)),
            TokenSinkResult::Plaintext => self.decode_entities.set(false),
            _ => {}
        }
        result
    }
    fn end(&self) {
        self.tree.end()
    }
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.tree
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

fn flatten(root: &Handle, output: &mut Vec<Token>) {
    let mut pending = vec![(root.clone(), false, 0)];
    while let Some((node, closing, fallback)) = pending.pop() {
        match &node.data {
            Data::Document => {
                for child in node.children.borrow().iter().rev() {
                    pending.push((child.clone(), false, fallback));
                }
            }
            Data::Ignored => {}
            Data::Text(units) => {
                let units = units.borrow();
                let mut at = 0;
                while at < units.len() {
                    let source = units[at].source.clone();
                    let mut text = units[at].value.to_string();
                    at += 1;
                    while source.is_some() && at < units.len() && units[at].source == source {
                        text.push(units[at].value);
                        at += 1;
                    }
                    output.push(Token {
                        range: source.clone().unwrap_or(fallback..fallback),
                        kind: TokenKind::MappedText {
                            text,
                            mapped: source.is_some(),
                        },
                    });
                }
            }
            Data::Element { name, attrs, .. } => {
                let range = node.source.clone().unwrap_or(fallback..fallback);
                let tag = Tag {
                    name: name.local.to_string(),
                    end: false,
                    attributes: attrs
                        .borrow()
                        .iter()
                        .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                        .collect(),
                };
                if closing {
                    let end = node.end.borrow().clone().unwrap_or_else(|| {
                        let at = output.last().map_or(range.end, |token| token.range.end);
                        at..at
                    });
                    output.push(Token {
                        range: end,
                        kind: TokenKind::Tag(Tag { end: true, ..tag }),
                    });
                } else {
                    output.push(Token {
                        range: range.clone(),
                        kind: TokenKind::Tag(tag),
                    });
                    pending.push((node.clone(), true, fallback));
                    for child in node.children.borrow().iter().rev() {
                        pending.push((child.clone(), false, range.end));
                    }
                }
            }
        }
    }
}

pub(super) fn tokens(input: &str) -> Vec<Token> {
    super::work_statistics::record(|stats| {
        stats.html_tree_tokenization_calls += 1;
        stats.html_tree_tokenized_bytes += input.len();
    });
    let context = Rc::new(RefCell::new(Context::default()));
    let document = Node::new(Data::Document, None);
    let sink = Sink {
        document: document.clone(),
        context,
        elements: RefCell::new(Vec::new()),
    };
    let queue = Rc::new(BufferQueue::default());
    queue.push_back(StrTendril::from(input));
    let tree = TreeBuilder::new(
        sink,
        html5ever::tree_builder::TreeBuilderOpts {
            scripting_enabled: true,
            ..Default::default()
        },
    );
    let tracker = Tracking {
        tree,
        queue: queue.clone(),
        input,
        cursor: Cell::new(0),
        entity: RefCell::new(VecDeque::new()),
        decode_entities: Cell::new(true),
    };
    let tokenizer = Tokenizer::new(
        tracker,
        tokenizer::TokenizerOpts {
            discard_bom: false,
            ..Default::default()
        },
    );
    while !matches!(tokenizer.feed(&queue), html5ever::TokenizerResult::Done) {}
    tokenizer.end();
    let mut output = Vec::new();
    flatten(&document, &mut output);
    // Tear down iteratively too: deeply nested external input must not make
    // Rc's recursive child destruction overflow the editor's stack.
    for node in tokenizer.sink.tree.sink.elements.borrow().iter() {
        node.children.borrow_mut().clear();
    }
    document.children.borrow_mut().clear();
    output
}
