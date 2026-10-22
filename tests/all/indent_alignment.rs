use viem_core::document::{Document, Encoding, Format, FileFormat};
use viem_core::layout::*;

fn fixture(text: &str, wrap: bool) -> (Document, LayoutEngine<MockTextMeasurementProvider>, ViewLayout) {
    let doc = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600., 200.);
    view.set_wrap(wrap);
    view.set_whitespace_presentation(WhitespacePresentationOptions {
        align_indents_to_previous_line: true, ..Default::default()
    }, Format::Code, 4).unwrap();
    (doc,engine,view)
}
fn x(snapshot: &LayoutSnapshot, at: usize) -> f32 {
    let point = snapshot.caret_point(at, BoundaryAffinity::Downstream).or_else(|_| snapshot.caret_point(at, BoundaryAffinity::Upstream)).unwrap();
    snapshot.caret_geometry(point).unwrap().rect.x
}
fn close(a:f32,b:f32) { assert!((a-b).abs()<0.02, "{a} != {b}"); }

#[test]
fn proportional_columns_align_in_wrapped_and_unwrapped_code() {
    let text = "  if (a == b ||\n      c == d) {\n      result();\n  }";
    for wrap in [false,true] {
        let (doc,mut engine,mut view) = fixture(text,wrap);
        engine.relayout(&doc,&mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        close(x(snapshot,text.find('a').unwrap()),x(snapshot,text.find('c').unwrap()));
        close(x(snapshot,text.find('a').unwrap()),x(snapshot,text.find("result").unwrap()));
        assert_eq!(doc.source_bytes(),text.as_bytes());
        let aligned = x(snapshot,text.find('c').unwrap());
        view.set_whitespace_presentation(Default::default(),Format::Code,4).unwrap();
        engine.relayout(&doc,&mut view).unwrap();
        assert!((x(view.snapshot().unwrap(),text.find('c').unwrap())-aligned).abs()>1.);
    }
}

#[test]
fn native_font_trait_and_code_family_gate_alignment() {
    let text="  if (a\n      c";
    for (format,monospaced) in [(Format::Code,true),(Format::PlainText,false),(Format::Markdown,false)] {
        let doc=Document::from_bytes(text.as_bytes().to_vec(),Encoding::Utf8,format).unwrap();
        let (_,mut engine,mut view)=fixture(text,false);
        engine.provider_mut().set_default_font_is_monospaced(monospaced);
        let options=WhitespacePresentationOptions { align_indents_to_previous_line:true,..Default::default() };
        view.set_whitespace_presentation(options,format,4).unwrap();
        engine.relayout(&doc,&mut view).unwrap();
        let on=view.snapshot().unwrap().rows.clone();
        view.set_whitespace_presentation(Default::default(),format,4).unwrap();
        engine.relayout(&doc,&mut view).unwrap();
        let off=&view.snapshot().unwrap().rows;
        assert_eq!(on.iter().map(|r|r.width).collect::<Vec<_>>(),off.iter().map(|r|r.width).collect::<Vec<_>>());
        for (on,off) in on.iter().zip(off.iter()) { assert_eq!(on.clusters,off.clusters); }
    }
}

#[test]
fn nested_and_short_anchor_lines_extend_with_whitespace_units() {
    let text="  if (a\n      call(W\n           nested\n      peer\n  x\n        beyond";
    let (doc,mut engine,mut view)=fixture(text,false);
    engine.relayout(&doc,&mut view).unwrap();
    let s=view.snapshot().unwrap();
    close(x(s,text.find('a').unwrap()),x(s,text.find("call").unwrap()));
    close(x(s,text.find('W').unwrap()),x(s,text.find("nested").unwrap()));
    close(x(s,text.find("call").unwrap()),x(s,text.find("peer").unwrap()));
    let end=text.find("\n        beyond").unwrap();
    close(x(s,text.find("beyond").unwrap()),x(s,end)+5.*s.whitespace_unit);
}

#[test]
fn distant_regional_anchor_and_equal_chains_need_bounded_shaping() {
    let text=format!("  if (a\n{}      c", "            deeper\n".repeat(50_000));
    let (doc,mut engine,mut view)=fixture(&text,false);
    let job=prepare_layout_job(&doc,&mut view,inspect_layout_provider(&engine),LayoutJobId(1),
        LayoutJobPriority::NewlyExposedRows,LayoutJobRegion::Viewport(ViewportLayoutRegion::new(50_001..50_002,0.,100.).unwrap()),LayoutCancellationToken::new()).unwrap();
    assert!(job.captured_text_len()<100);
    let candidate=compute_layout_job(&mut engine,&job,LayoutExecutionContext::WorkerPool).unwrap();
    assert!(engine.provider().request_calls()<20);
    install_layout_job(&mut view,LayoutInstallTarget {document_id:doc.id(),document_revision:doc.revision(),
        measurement_environment_id:engine.provider().measurement_environment_id(),metrics_generation:engine.provider().metrics_generation()},candidate).unwrap();
    let distant=x(view.snapshot().unwrap(),text.rfind('c').unwrap());
    let (small,mut small_engine,mut small_view)=fixture("  if (a\n      c",false);
    small_engine.relayout(&small,&mut small_view).unwrap();
    close(distant,x(small_view.snapshot().unwrap(),14));
}

#[test]
fn local_edits_undo_and_reindent_match_fresh_layout() {
    let (mut doc,mut engine,mut view)=fixture("  if (a\n      c\n          deep\n      peer\nroot",true);
    for edit in [(2,"W"),(18,"\n  inserted\n"),(0,"    ")] {
        engine.relayout(&doc,&mut view).unwrap();
        doc.insert(edit.0,edit.1).unwrap();
        engine.relayout(&doc,&mut view).unwrap();
        let mut fresh=LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut fresh_view=ViewLayout::new(600.,200.);
        fresh_view.set_whitespace_presentation(view.whitespace_presentation().clone(),Format::Code,4).unwrap();
        fresh.relayout(&doc,&mut fresh_view).unwrap();
        for (a,b) in view.snapshot().unwrap().rows.iter().zip(fresh_view.snapshot().unwrap().rows.iter()) {
            close(a.width,b.width);
            assert_eq!(a.clusters,b.clusters);
        }
        assert!(doc.undo());
    }
}

#[test]
fn tabs_unicode_and_mac_literal_lf_keep_semantic_lines() {
    for text in ["\tif (a\n\t    c", "  e\u{301}界a\n     c", "  🙂ab\n    c"] {
        let (doc,mut engine,mut view)=fixture(text,false);
        engine.relayout(&doc,&mut view).unwrap();
        assert_eq!(doc.source_bytes(),text.as_bytes());
        assert!(view.snapshot().unwrap().rows.iter().all(|r|r.width.is_finite()));
    }
    let bytes=b"  if (a\rc\nliteral\r      child".to_vec();
    let doc=Document::from_bytes_with_file_format(bytes.clone(),Encoding::Utf8,Format::Code,FileFormat::Mac).unwrap();
    let (_,mut engine,mut view)=fixture("",false);
    engine.relayout(&doc,&mut view).unwrap();
    assert_eq!(view.snapshot().unwrap().rows.len(),3);
    assert_eq!(doc.source_bytes(),bytes);
}
