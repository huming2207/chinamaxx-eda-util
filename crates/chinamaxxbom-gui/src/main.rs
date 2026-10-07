mod view;
use gpui::*;
use gpui_component::Root;
fn main() {
    let initial = std::env::args().nth(1);
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1280.), px(850.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(650.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("ChinamaxxBOM — Eagle · KiCad".into()),
                        ..Default::default()
                    }),
                    app_id: Some("chinamaxxbom".into()),
                    ..Default::default()
                },
                |window, cx| {
                    gpui_component::Theme::sync_system_appearance(Some(window), cx);
                    let view = cx.new(|cx| view::BomView::new(initial.clone(), window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Cannot open the GPUI window; check the display and Vulkan drivers");
            cx.activate(true);
        });
}
