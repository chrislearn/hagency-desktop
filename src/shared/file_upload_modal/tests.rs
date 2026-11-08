use super::*;
use makepad_widgets::makepad_draw::cx_draw::CxDraw;

fn fixture() -> (Cx, WidgetRef) {
    let mut cx = Cx::new(Box::new(|_, _| {}));
    let modal = cx.with_vm(|vm| {
        makepad_widgets::script_mod(vm);
        crate::app::register_widgets(vm);
        vm.bx.captured_errors = Some(Vec::new());
        let value = script_eval!(vm, { mod.widgets.FileUploadModal {} });
        let modal = WidgetRef::script_from_value(vm, value);
        modal.register_as_ui_root(vm);
        assert!(vm.take_errors().is_empty());
        modal
    });
    (cx, modal)
}

fn draw(cx: &mut Cx, modal: &WidgetRef, size: Vec2d) {
    cx.redraw_all();
    let pass = DrawPass::new(cx);
    pass.set_size(cx, size);
    let mut list = DrawList2d::new(cx);
    let event = DrawEvent::default();
    let mut draw = CxDraw::new(cx, &event);
    let mut cx = Cx2d::new(&mut draw);
    cx.begin_pass(&pass, None);
    list.begin_always(&mut cx);
    cx.begin_root_turtle(size, Layout::flow_overlay());
    modal.draw_all(&mut cx, &mut Scope::empty());
    cx.end_pass_sized_turtle();
    list.end(&mut cx);
    cx.end_pass(&pass);
}

#[test]
fn upload_preview_fits_content_and_keeps_actions_inside_the_window() {
    let (mut cx, modal) = fixture();
    for size in [dvec2(1440.0, 900.0), dvec2(800.0, 600.0), dvec2(390.0, 640.0)] {
        for text in [false, true] {
            let preview = if text {
                FilePreview::Text(TextPreview { content: "Preview line\n".repeat(80), truncated: false, is_code: false })
            } else {
                FilePreview::None
            };
            modal.borrow_mut::<FileUploadModal>().unwrap().show_preview(&mut cx, preview);
            draw(&mut cx, &modal, size);
            let card = modal.area().rect(&cx);
            let upload = modal.button(&mut cx, ids!(upload_button)).area().rect(&cx);
            assert!(card.size.x > 250.0 && card.size.x <= 560.0, "{size:?}: {card:?}");
            assert!(card.size.y < 580.0 && card.size.y < size.y, "{size:?}: {card:?}");
            assert!(upload.size.x > 0.0 && upload.size.y > 0.0);
            assert!(card.contains(upload.pos) && card.contains(upload.pos + upload.size), "Upload button outside card: {card:?} {upload:?}");
            assert!(upload.pos.y + upload.size.y <= size.y, "Upload button outside window: {upload:?}");
            if !text { assert!(card.size.y < 430.0, "Document without preview should stay compact: {card:?}"); }
        }
    }
}

