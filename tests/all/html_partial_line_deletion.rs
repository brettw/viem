use viem_core::command::{CommandInterpreter, InputEvent};
use viem_core::{Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn list_hard_line_deletion_preserves_surviving_content() {
    for (source, keys, expected) in [
        (
            "<ol><li>First<br></li><li>Second</li></ol>",
            "1Gdd",
            "\nSecond",
        ),
        (
            "<ol><li>First<br></li><li>Second</li></ol>",
            "2Gdd",
            "First\nSecond",
        ),
        (
            "<ol><li>First<br>More</li><li>Second</li></ol>",
            "1Gdd",
            "More\nSecond",
        ),
        (
            "<ol><li>First<br>More</li><li>Second</li></ol>",
            "2Gdd",
            "First\nSecond",
        ),
        (
            "<ol><li><p>First</p><p>More</p></li><li>Second</li></ol>",
            "1Gdd",
            "More\nSecond",
        ),
        (
            "<ol><li><p>First</p><p>More</p></li><li>Second</li></ol>",
            "2Gdd",
            "First\nSecond",
        ),
        (
            "<ol><li>First<ul><li>Child</li></ul></li><li>Second</li></ol>",
            "1Gdd",
            "Child\nSecond",
        ),
    ] {
        let mut document = html(source);
        let mut commands = CommandInterpreter::new();
        for key in keys.chars() {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap_or_else(|error| panic!("{keys}: {source}: {error:?}"));
        }
        assert_eq!(document.text(), expected, "{keys}: {source}");
        let changed = document.source_bytes();
        assert_eq!(
            html(std::str::from_utf8(&changed).unwrap()).text(),
            expected
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn partial_line_delete_preserves_unselected_markup_and_register_extent() {
    let source = "<ol start='4'><li data-keep='yes'><b>First</b><br><i>More</i><!--keep--></li><li>Second</li></ol>";
    let mut document = html(source);
    let mut commands = CommandInterpreter::new();
    for key in "2G\"add".chars() {
        commands
            .handle(&mut document, InputEvent::key(key))
            .unwrap();
    }
    assert_eq!(document.text(), "First\nSecond");
    assert_eq!(commands.register('a').unwrap().text, "More\n");
    assert_eq!(
        std::str::from_utf8(&document.source_bytes()).unwrap(),
        "<ol start='4'><li data-keep='yes'><b>First</b><!--keep--></li><li>Second</li></ol>"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn counted_line_delete_removes_complete_owner_and_keeps_survivor_numbering() {
    let source = "<ol start='4'><li>First<br>More</li><li>Second</li></ol><!--keep-->";
    let mut document = html(source);
    let mut commands = CommandInterpreter::new();
    for key in "2dd".chars() {
        commands
            .handle(&mut document, InputEvent::key(key))
            .unwrap();
    }
    assert_eq!(document.text(), "Second");
    assert_eq!(commands.register('\"').unwrap().text, "First\nMore\n");
    let lists = document.projection().list_structure();
    assert_eq!(lists.lists.len(), 1);
    assert_eq!(lists.lists[0].items.len(), 1);
    assert_eq!(lists.lists[0].items[0].ordinal, 5);
    assert!(std::str::from_utf8(&document.source_bytes())
        .unwrap()
        .ends_with("<!--keep-->"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn final_intra_paragraph_line_delete_maps_the_removed_native_break() {
    let document = html("<ol><li>First<br></li><li>Second</li></ol>");
    let prepared = document
        .prepare_model_request(viem_core::document::ModelRequest::DeleteLines {
            document: document.id(),
            revision: document.revision(),
            range: 6..7,
        })
        .unwrap();
    assert_eq!(prepared.summary().formatted_splices().len(), 1);
    assert_eq!(prepared.summary().formatted_splices()[0].old_range(), 5..6);
}
