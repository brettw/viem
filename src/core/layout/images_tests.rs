//! Image dimensions are platform data; layout owns containment and caret geometry.
use super::*;
use crate::document::{Document, Encoding, Format, ImageEditIntent};

struct Images {
    inner: MockTextMeasurementProvider,
    destinations: Vec<String>,
    observed_bytes: usize,
    resource_width: f32,
}
impl Images {
    fn new() -> Self { Self { inner: MockTextMeasurementProvider::new(), destinations: Vec::new(), observed_bytes: 0, resource_width: 600. } }
}
impl TextMeasurementProvider for Images {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId { self.inner.measurement_environment_id() }
    fn metrics_generation(&self) -> MetricsGeneration { self.inner.metrics_generation() }
    fn render_run_policy(&self) -> Option<RenderRunPolicy> { self.inner.render_run_policy() }
    fn shape_batch(&mut self, requests: &[ShapeRequest<'_>]) -> Result<Vec<ShapedFragment>, MeasurementError> {
        let mut responses = self.inner.shape_batch(requests)?;
        for (request, response) in requests.iter().zip(&mut responses) {
            self.observed_bytes += request.text.len();
            for image in request.inline_images {
                self.destinations.push(image.destination.clone());
                let Some(cluster) = response.clusters.iter_mut().find(|c| c.text_range == image.text_range) else { continue; };
                let (width, height) = if image.destination == "small.png" { (40.,20.) }
                    else if image.destination == "tall.png" { (100.,500.) }
                    else { (self.resource_width,self.resource_width / 2.) };
                cluster.advance = width * request.scale;
                cluster.metrics = TextMetrics { ascent: height * request.scale, descent: 0., leading: 0. };
                cluster.typographic_bounds = ShapedBounds { x:0.,y:-cluster.metrics.ascent,width:cluster.advance,height:cluster.metrics.ascent };
                cluster.ink_bounds = cluster.typographic_bounds;
                for stop in &mut cluster.caret_stops {
                    stop.inline_offset = if (stop.text_offset == image.text_range.start) == (cluster.bidi_level % 2 == 0) { 0. } else { cluster.advance };
                }
            }
        }
        Ok(responses)
    }
}
fn document(source: &str) -> Document { Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap() }
fn image<'a>(snapshot: &'a LayoutSnapshot, at: usize) -> &'a PositionedCluster {
    snapshot.rows.iter().flat_map(|row| &row.clusters).find(|c| c.text_range == (at..at+3)).unwrap()
}
fn region(document: &Document, view: &mut ViewLayout, engine: &mut LayoutEngine<Images>, id:u64, lines:std::ops::Range<usize>) -> LayoutJobCandidate {
    let request=prepare_layout_job(document, view, inspect_layout_provider(engine), LayoutJobId(id),
        LayoutJobPriority::ChangedVisibleRows, LayoutJobRegion::HardLines(HardLineLayoutRegion::new(lines).unwrap()),
        LayoutCancellationToken::new()).unwrap();
    compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap()
}
#[test]
fn images_keep_intrinsic_aspect_shrink_to_content_and_have_atomic_hit_geometry() {
    let document=document("![wide](wide.png) text ![small](small.png)");
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(220.,300.);
    view.set_insets(EdgeInsets {left:10.,right:10.,..Default::default()});
    engine.relayout(&document,&mut view).unwrap();
    let snapshot=view.snapshot().unwrap();
    let wide=image(snapshot,0);
    assert_eq!((wide.advance,wide.typographic_bounds.height),(200.,100.));
    assert_eq!(wide.typographic_bounds,wide.ink_bounds);
    let small=image(snapshot,document.text().rfind('\u{fffc}').unwrap());
    assert_eq!((small.advance,small.typographic_bounds.height),(40.,20.));
    for fraction in [0.1,0.5,0.9] {
        let point=snapshot.hit_test_character(LayoutPoint {x:wide.x+wide.advance*fraction,y:wide.typographic_bounds.y+30.}).unwrap();
        assert_eq!(point.text_offset,0);
    }
    let wide_row=&snapshot.rows[0];
    assert!(wide_row.height()>=100.);
    assert!(wide_row.carets.iter().all(|caret| caret.point.text_offset==0 || caret.point.text_offset>=3));
    assert_eq!(document.source_bytes(),b"![wide](wide.png) text ![small](small.png)");
}
#[test]
fn resizing_reuses_intrinsic_shapes_and_never_upscales() {
    let document=document("![wide](wide.png)");
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(200.,300.);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),0).advance,200.);
    let requests=engine.provider().inner.request_calls();
    view.resize(900.,300.);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),0).advance,600.);
    assert_eq!(engine.provider().inner.request_calls(),requests);
    view.resize(100.,300.);
    view.set_wrap(false);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),0).typographic_bounds.height,50.);
    assert_eq!(engine.provider().inner.request_calls(),requests);
}
#[test]
fn url_only_edits_and_resource_generations_retire_cached_geometry() {
    let mut document=document("![wide](wide.png)");
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(700.,300.);
    let first=region(&document,&mut view,&mut engine,1,0..1);
    install_layout_job(&mut view,LayoutInstallTarget { document_id:document.id(),document_revision:document.revision(),
        measurement_environment_id:engine.provider().measurement_environment_id(),metrics_generation:engine.provider().metrics_generation() },first).unwrap();
    let (edit,_)=document.prepare_image_edit(document.id(),document.revision(),ImageEditIntent::Edit {
        range:0..3,text:"wide".into(),destination:"small.png".into() }).unwrap();
    document.commit_model_transaction(edit).unwrap();
    assert_eq!(document.text(),"\u{fffc}");
    let second=region(&document,&mut view,&mut engine,2,0..1);
    let cluster=&second.regional_snapshot().lines()[0].rows()[0].clusters[0];
    assert_eq!((cluster.advance,cluster.typographic_bounds.height),(40.,20.));
    assert!(engine.provider().destinations.iter().any(|s|s=="small.png"));
    assert!(document.undo());
    engine.provider_mut().resource_width=320.;
    engine.provider_mut().inner.set_metrics_generation(MetricsGeneration(2));
    let third=region(&document,&mut view,&mut engine,3,0..1);
    let cluster=&third.regional_snapshot().lines()[0].rows()[0].clusters[0];
    assert_eq!((cluster.advance,cluster.typographic_bounds.height),(320.,160.));
    assert_eq!(cluster.render_run.as_ref().unwrap().metrics_generation,MetricsGeneration(2));
}
#[test]
fn large_image_document_queries_and_shapes_only_requested_region() {
    let source=(0..10_000).map(|i|format!("![image {i}](local-{i}.png)\n\n")).collect::<String>();
    let document=document(&source);
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(200.,300.);
    let candidate=region(&document,&mut view,&mut engine,1,7000..7003);
    assert_eq!(candidate.regional_snapshot().lines().len(),3);
    assert_eq!(engine.provider().destinations.len(),3);
    assert!(engine.provider().observed_bytes<=9);
    for line in candidate.regional_snapshot().lines() {
        let cluster=&line.rows()[0].clusters[0];
        assert_eq!((cluster.advance,cluster.typographic_bounds.height),(200.,100.));
    }
}
#[test]
fn source_view_shapes_literal_image_syntax_and_never_requests_a_resource() {
    let document=Document::from_bytes(b"![alt](https://example.test/image.png)".to_vec(),Encoding::Utf8,Format::MarkdownSource).unwrap();
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(1000.,300.);
    engine.relayout(&document,&mut view).unwrap();
    assert!(engine.provider().destinations.is_empty());
    assert_eq!(document.text(),"![alt](https://example.test/image.png)");
}

#[test]
fn image_edges_remain_atomic_next_to_combining_marks() {
    let document=document("![wide](wide.png)\u{301}x");
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(700.,300.);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),0).text_range,0..3);
    assert!(view.snapshot().unwrap().rows[0].carets.iter().any(|caret|caret.point.text_offset==3));
}
#[test]
fn resource_change_rejects_an_in_flight_image_layout() {
    let document=document("![wide](wide.png)");
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(700.,300.);
    let candidate=region(&document,&mut view,&mut engine,1,0..1);
    engine.provider_mut().inner.set_metrics_generation(MetricsGeneration(2));
    assert!(matches!(install_layout_job(&mut view,LayoutInstallTarget {
        document_id:document.id(),document_revision:document.revision(),
        measurement_environment_id:engine.provider().measurement_environment_id(),
        metrics_generation:engine.provider().metrics_generation(),
    },candidate),Err(LayoutJobInstallRejection::StaleMetrics {..})));
    assert!(view.snapshot().is_none());
}
#[test]
fn table_images_resize_without_reusing_stale_intrinsic_widths() {
    let document=document("| Image |\n| --- |\n| ![wide](wide.png) |\n");
    let image_at=document.text().find('\u{fffc}').unwrap();
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(700.,500.);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),image_at).advance,600.);
    view.resize(180.,500.);
    engine.relayout(&document,&mut view).unwrap();
    assert_eq!(image(view.snapshot().unwrap(),image_at).advance,180.);
    assert!(view.snapshot().unwrap().tables()[0].rect.width<300.);
}
#[test]
fn destination_only_change_retires_offscreen_table_width_contribution() {
    let mut document=document("| Image |\n| --- |\n| ![wide](wide.png) |\n| ![small](small.png) |\n");
    let image_at=document.text().find('\u{fffc}').unwrap();
    let mut engine=LayoutEngine::new(Images::new());
    let mut view=ViewLayout::new(700.,500.);
    engine.relayout(&document,&mut view).unwrap();
    assert!(view.snapshot().unwrap().tables()[0].rect.width>=600.);
    let old_revision=document.revision();
    let old_lines=document.line_count();
    let (edit,_)=document.prepare_image_edit(document.id(),document.revision(),ImageEditIntent::Edit {
        range:image_at..image_at+3,text:"wide".into(),destination:"small.png".into() }).unwrap();
    document.commit_model_transaction(edit).unwrap();
    engine.rebase_document_change(&super::engine::DocumentLayoutChange::Local {
        old_revision,new_revision:document.revision(),old_hull:image_at..image_at+3,new_hull:image_at..image_at+3,
        old_line_count:old_lines,new_line_count:document.line_count(),invalidated_lines:1..2,
    });
    // Request another row; the previous widest image is outside this viewport.
    let candidate=region(&document,&mut view,&mut engine,1,2..3);
    assert!(candidate.regional_snapshot().lines()[0].rows()[0].table_cell.as_ref().unwrap().table_width<150.);
}
