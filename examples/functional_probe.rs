use viem_core::command::{InputEvent, Key};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};
use serde_json::{json, Value};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let format = match value["format"].as_str().unwrap_or("plain") {
        "plain" => Format::PlainText,
        "markdown" => Format::Markdown,
        "markdown_source" => Format::MarkdownSource,
        "code" => Format::Code,
        other => panic!("unknown format {other}"),
    };
    let document = Document::from_bytes(value["source"].as_str().unwrap().as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), value["width"].as_f64().unwrap_or(180.) as f32, 80.);
    if let Some(at) = value["cursor"].as_u64() {
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(), text_offset:at as usize, affinity:viem_core::document::BoundaryAffinity::Downstream, extend_selection:false }).unwrap();
    }
    println!("{}",json!({"phase":"before","text":core.document().text(),"blocks":format!("{:?}",core.document().projection().blocks()),"provenance":format!("{:?}",core.document().projection().provenance())}));
    for action in value["actions"].as_array().unwrap_or(&Vec::new()) {
        let input = if let Some(text)=action["text"].as_str() { InputEvent::text(text) } else {
            InputEvent::Key(match action["key"].as_str().unwrap() { "Escape"=>Key::Escape,"Enter"=>Key::Enter,"Backspace"=>Key::Backspace,key=>Key::Char(key.chars().next().unwrap()) })
        };
        let result=core.handle(view,CoreEvent::Input(input));
        println!("{}",json!({"action":action,"result":format!("{result:?}"),"text":core.document().text(),"source":String::from_utf8_lossy(&core.document().source_bytes()),"cursor":core.command_state(view).unwrap().cursor()}));
    }
}
