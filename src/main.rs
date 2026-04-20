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
