use makepad_widgets::{makepad_draw::cx_draw::CxDraw, *};

fn draw(cx: &mut Cx, settings: &WidgetRef, width: f64) {
    let size = dvec2(width, 900.0);
    cx.redraw_all();
    let pass = DrawPass::new(cx);
    pass.set_size(cx, size);
    let mut list = DrawList2d::new(cx);
    let overlay = Overlay {
        draw_list: DrawList::new(cx),
    };
    let event = DrawEvent::default();
    let mut draw = CxDraw::new(cx, &event);
    let mut cx = Cx2d::new(&mut draw);
    cx.begin_pass(&pass, None);
    list.begin_always(&mut cx);
    overlay.begin(&mut cx);
    cx.begin_root_turtle(size, Layout::flow_overlay());
    settings.draw_walk_all(&mut cx, &mut Scope::empty(), Walk::fill());
    overlay.end(&mut cx);
    cx.end_pass_sized_turtle();
    list.end(&mut cx);
    cx.end_pass(&pass);
}

#[test]
fn account_settings_reflow_with_the_pane() {
    let mut cx = Cx::new(Box::new(|_, _| {}));
    let settings = cx.with_vm(|vm| {
        makepad_widgets::script_mod(vm);
        crate::app::register_widgets(vm);
        let value = script_eval!(vm, { mod.widgets.SettingsScreen {} });
        let settings = WidgetRef::script_from_value(vm, value);
        settings.register_as_ui_root(vm);
        settings
    });
    let banner = settings.widget(&cx, ids!(verification_banner_verified));
    let recovery = settings.widget(&cx, ids!(encryption_recovery));
    let status = recovery.label(&cx, ids!(status));
    banner.set_visible(&mut cx, true);
    recovery
        .widget(&cx, ids!(restore_section))
        .set_visible(&mut cx, true);
    status.set_text(&mut cx, "Device verification: Ready · Key recovery: Identity keys available on this device · Room key backup: Connected");
    let restore = recovery.widget(&cx, ids!(restore));
    let upload = recovery.widget(&cx, ids!(upload));
    let description = recovery.label(&cx, ids!(restore_description));
    let mut heights = Vec::new();
    for width in [1280.0, 816.0, 375.0, 816.0] {
        draw(&mut cx, &settings, width);
        let banner_rect = banner.area().rect(&cx);
        let recovery_rect = recovery.area().rect(&cx);
        assert!((banner_rect.size.x - recovery_rect.size.x).abs() < 1.0);
        assert!(banner_rect.size.x > width.min(700.0) - 40.0);
        assert!(recovery_rect.pos.x + recovery_rect.size.x <= width);
        for label in [&status, &description] {
            let ink = label.borrow().unwrap().draw_text.area();
            let bounds = ink.rect_union(&cx);
            let visible = ink.clipped_rect_union(&cx);
            assert!(bounds.size.x > 0.0 && bounds.size.y > 0.0);
            assert!(
                (bounds.size - visible.size).length() < 1.0,
                "{bounds:?} vs {visible:?}"
            );
        }
        let first = restore.area().rect(&cx);
        let last = upload.area().rect(&cx);
        if width == 375.0 {
            assert!(last.pos.y > first.pos.y + first.size.y);
        } else {
            assert_eq!(first.pos.y, last.pos.y);
        }
        heights.push((banner_rect.size.y, description.area().rect(&cx).size.y));
    }
    assert_eq!(heights[0], heights[1]);
    assert!(heights[2].0 > heights[1].0 && heights[2].1 > heights[1].1);
    assert_eq!(heights[3], heights[1]);

    banner.set_visible(&mut cx, false);
    let unverified = settings.widget(&cx, ids!(verification_banner_unverified));
    unverified.set_visible(&mut cx, true);
    draw(&mut cx, &settings, 816.0);
    assert_eq!(
        unverified.area().rect(&cx).size.x,
        recovery.area().rect(&cx).size.x
    );
}
