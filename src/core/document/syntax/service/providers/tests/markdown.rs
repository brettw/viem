use super::*;

fn assert_style(result: &SyntaxResult, text: &str, token: &str, style: &str) {
    let at = text.find(token).unwrap();
    assert_eq!(
        result
            .runs
            .iter()
            .find(|run| run.range.contains(&at))
            .map(|run| run.name.as_str()),
        Some(style),
        "{token}: {:?}",
        result.runs,
    );
}

#[test]
fn markdown_blocks_inline_tables_and_fenced_languages_share_literal_coordinates() {
    let _registry = treesitter::package_registry_test_guard();
    let text = concat!(
        "# Heading\n\n",
        "**café** and *italic* with ~~gone~~, `raw`, [label](https://example.com), &amp;\n\n",
        "- [x] done\n\n",
        "> **across\n> lines**\n\n",
        "| Column | Value |\n| --- | --- |\n| **cell** | *other* |\n\n",
        "```rust\nfn main() { let answer = 42; }\n```\n\n",
        "```not_a_language\nplain body\n```\n",
    );
    let req = request(text, "markdown", 1);
    let result = finish(&mut BackendProvider::default(), &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    for (token, style) in [
        ("Heading", "Markup.heading.1"),
        ("café", "Markup.strong"),
        ("italic", "Markup.italic"),
        ("gone", "Markup.strikethrough"),
        ("raw", "Markup.raw"),
        ("label", "Markup.link.label"),
        ("https://example.com", "Markup.link.url"),
        ("&amp;", "Character.special"),
        ("[x]", "Markup.list.checked"),
        ("across", "Markup.strong"),
        ("lines", "Markup.strong"),
        ("Column", "Markup.heading"),
        ("cell", "Markup.strong"),
        ("other", "Markup.italic"),
        ("fn main", "Keyword.function"),
        ("main()", "Function"),
        ("let answer", "Keyword"),
        ("42", "Number"),
        ("plain body", "Markup.raw.block"),
    ] {
        assert_style(&result, text, token, style);
    }
    assert!(!result.runs.iter().any(|run| run.name.as_str() == "Conceal"));
    assert_eq!(req.input.slice(0..text.len()).unwrap(), text);
    assert!(result
        .runs
        .windows(2)
        .all(|pair| pair[0].range.end <= pair[1].range.start));
}

#[test]
fn markdown_edits_to_inline_delimiters_and_fence_languages_match_fresh_analysis() {
    let _registry = treesitter::package_registry_test_guard();
    let mut provider = BackendProvider::default();
    // Language edits leave the fence's shape intact; removing its close changes
    // the dependency through EOF. Neither may retain old injected colors.
    for (index, text) in [
        "# Heading\n\n**word**\n\n```rust\nlet value = true;\n```\n\n*tail*\n",
        "# Heading\n\n`word`\n\n```rust\nlet value = true;\n```\n\n*tail*\n",
        "# Heading\n\n`word`\n\n```python\nlet value = true;\n```\n\n*tail*\n",
        "# Heading\n\n`word`\n\n```unknown\nlet value = true;\n```\n\n*tail*\n",
        "# Heading\n\n`word`\n\n```rust\nlet value = true;\n\n*tail*\n",
        "# Heading\n\n**word**\n\n```rust\nlet value = true;\n```\n\n*tail*\n",
    ]
    .into_iter()
    .enumerate()
    {
        let req = request(text, "markdown", index as u64 + 1);
        let incremental = finish(&mut provider, &req);
        let fresh = finish(&mut BackendProvider::default(), &req);
        assert_eq!(
            incremental.coverage,
            Coverage::Exact,
            "{:?}",
            incremental.diagnostics
        );
        assert_eq!(fresh.coverage, Coverage::Exact, "{:?}", fresh.diagnostics);
        assert_eq!(incremental.runs, fresh.runs, "{text}");
        assert_eq!(incremental.diagnostics, fresh.diagnostics);
    }
}

#[test]
fn markdown_distant_scroll_queries_only_visible_inline_children_and_reuses_block_tree() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "**word** and *other*\n\n".repeat(2_000);
    let mut req = request(&text, "markdown", 1);
    req.range = 0..20;
    let mut provider = BackendProvider::default();
    let first = finish(&mut provider, &req);
    assert_eq!(first.coverage, Coverage::Exact, "{:?}", first.diagnostics);
    assert_style(&first, &text, "word", "Markup.strong");
    assert_eq!(provider.children.len(), 1);
    let retained = provider
        .primary
        .as_ref()
        .unwrap()
        .completed()
        .unwrap()
        .clone();
    let parse_work = (provider.parse_slices, provider.parse_progress);
    req.range = text.len() - 22..text.len();
    let distant = finish(&mut provider, &req);
    assert_eq!(
        distant.coverage,
        Coverage::Exact,
        "{:?}",
        distant.diagnostics
    );
    assert_eq!((provider.parse_slices, provider.parse_progress), parse_work);
    assert_eq!(provider.children.len(), 1);
    assert_eq!(
        provider
            .primary
            .as_ref()
            .unwrap()
            .completed()
            .unwrap()
            .input_identity(),
        retained.input_identity()
    );
    let fresh = finish(&mut BackendProvider::default(), &req);
    assert_eq!(distant.runs, fresh.runs);
}
