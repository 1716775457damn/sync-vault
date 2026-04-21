mod app;
mod state;
mod syncer;
mod watcher;

fn main() {
    // On macOS, fall back to OpenGL if Metal fails (older Intel Mac / Rosetta)
    #[cfg(target_os = "macos")]
    if std::env::var("WGPU_BACKEND").is_err() {
        unsafe { std::env::set_var("WGPU_BACKEND", "metal,gl"); }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sync Vault")
            .with_inner_size([700.0, 520.0])
            .with_min_inner_size([500.0, 400.0])
            .with_icon(make_icon()),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native("Sync Vault", options, Box::new(|cc| {
        // ── Native-quality dark theme ─────────────────────────────────────────
        let mut visuals = egui::Visuals::dark();

        visuals.window_fill        = egui::Color32::from_rgb(18, 22, 26);
        visuals.panel_fill         = egui::Color32::from_rgb(24, 29, 35);
        visuals.faint_bg_color     = egui::Color32::from_rgb(30, 36, 44);
        visuals.extreme_bg_color   = egui::Color32::from_rgb(13, 16, 20);

        visuals.window_stroke      = egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 60, 72));
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 52, 64));

        let accent     = egui::Color32::from_rgb(52, 211, 153);  // emerald-400
        let accent_dim = egui::Color32::from_rgb(6,  78,  59);   // emerald-900

        visuals.widgets.inactive.bg_fill        = egui::Color32::from_rgb(34, 42, 52);
        visuals.widgets.inactive.bg_stroke       = egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 62, 76));
        visuals.widgets.inactive.corner_radius   = egui::CornerRadius::same(6);
        visuals.widgets.inactive.fg_stroke       = egui::Stroke::new(1.5, egui::Color32::from_rgb(175, 190, 205));

        visuals.widgets.hovered.bg_fill          = egui::Color32::from_rgb(44, 54, 68);
        visuals.widgets.hovered.bg_stroke        = egui::Stroke::new(1.0, egui::Color32::from_rgb(75, 95, 115));
        visuals.widgets.hovered.corner_radius    = egui::CornerRadius::same(6);
        visuals.widgets.hovered.fg_stroke        = egui::Stroke::new(1.5, egui::Color32::WHITE);
        visuals.widgets.hovered.expansion        = 1.0;

        visuals.widgets.active.bg_fill           = accent_dim;
        visuals.widgets.active.bg_stroke         = egui::Stroke::new(1.0, accent);
        visuals.widgets.active.corner_radius     = egui::CornerRadius::same(6);
        visuals.widgets.active.fg_stroke         = egui::Stroke::new(2.0, egui::Color32::WHITE);

        visuals.widgets.open.bg_fill             = egui::Color32::from_rgb(28, 36, 44);
        visuals.widgets.open.bg_stroke           = egui::Stroke::new(1.0, accent);
        visuals.widgets.open.corner_radius       = egui::CornerRadius::same(6);

        visuals.selection.bg_fill                = egui::Color32::from_rgba_unmultiplied(52, 211, 153, 50);
        visuals.selection.stroke                 = egui::Stroke::new(1.0, accent);
        visuals.hyperlink_color                  = accent;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 55, 68));
        visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };

        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.text_styles = [
            (egui::TextStyle::Small,     egui::FontId::proportional(11.5)),
            (egui::TextStyle::Body,      egui::FontId::proportional(13.5)),
            (egui::TextStyle::Button,    egui::FontId::proportional(13.0)),
            (egui::TextStyle::Heading,   egui::FontId::proportional(16.0)),
            (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
        ].into();
        style.spacing.item_spacing      = egui::vec2(8.0, 5.0);
        style.spacing.button_padding    = egui::vec2(10.0, 5.0);
        style.spacing.window_margin     = egui::Margin::same(12);
        style.spacing.indent            = 18.0;
        style.spacing.interact_size     = egui::vec2(40.0, 28.0);
        style.spacing.scroll.bar_width  = 6.0;
        style.spacing.scroll.bar_inner_margin = 2.0;
        cc.egui_ctx.set_style(style);

        let mut fonts = egui::FontDefinitions::default();
        let cjk: &[u8] = include_bytes!("../assets/NotoSansSC-Regular.otf");
        fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_static(cjk).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push("cjk".to_owned());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk".to_owned());
        cc.egui_ctx.set_fonts(fonts);
        Ok(Box::new(app::App::default()))
    })) {
        eprintln!("sync-vault failed to start: {e}");
        #[cfg(target_os = "macos")]
        {
            let msg = format!("sync-vault failed to start:\n{e}");
            let _ = std::process::Command::new("osascript")
                .args(["-e", &format!("display alert \"Sync Vault\" message \"{msg}\"")])
                .spawn();
        }
        std::process::exit(1);
    }
}

fn make_icon() -> egui::IconData {
    const S: usize = 32;
    let mut px = vec![0u8; S * S * 4];
    let set = |px: &mut Vec<u8>, x: i32, y: i32, r: u8, g: u8, b: u8| {
        if x >= 0 && y >= 0 && (x as usize) < S && (y as usize) < S {
            let i = (y as usize * S + x as usize) * 4;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    };
    // Two overlapping rectangles (sync arrows concept) in teal/green
    for y in 2i32..18 {
        for x in 2i32..20 {
            set(&mut px, x, y, 40, 180, 160);
        }
    }
    for y in 14i32..30 {
        for x in 12i32..30 {
            set(&mut px, x, y, 30, 140, 220);
        }
    }
    // Arrow right on top rect
    for i in 0i32..5 { set(&mut px, 14+i, 10, 255, 255, 255); }
    set(&mut px, 16, 8, 255, 255, 255);
    set(&mut px, 16, 12, 255, 255, 255);
    // Arrow left on bottom rect
    for i in 0i32..5 { set(&mut px, 16+i, 22, 255, 255, 255); }
    set(&mut px, 18, 20, 255, 255, 255);
    set(&mut px, 18, 24, 255, 255, 255);
    egui::IconData { rgba: px, width: S as u32, height: S as u32 }
}
