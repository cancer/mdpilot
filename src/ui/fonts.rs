use eframe::egui;

/// Install Japanese-capable fonts into egui's font definitions, falling back
/// to whatever ships with the OS. The release plan owns providing Windows
/// support (see `docs/plan.md` Phase 8); this implementation handles macOS.
pub fn install_japanese(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded: Vec<&'static str> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // AquaKana ships hiragana/katakana out of the box; Hiragino Sans GB
        // gives us the CJK ideograph coverage that Japanese borrows.
        loaded.extend(try_install_font(
            &mut fonts,
            "aqua_kana",
            "/System/Library/Fonts/AquaKana.ttc",
            FontSlot::ProportionalFront,
        ));
        loaded.extend(try_install_font(
            &mut fonts,
            "hiragino_sans_gb",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            FontSlot::ProportionalBack,
        ));
    }

    if loaded.is_empty() {
        tracing::warn!("no Japanese system fonts could be loaded; UI text will fall back to tofu",);
    } else {
        tracing::info!(fonts = ?loaded, "installed Japanese fonts");
    }

    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy)]
enum FontSlot {
    /// Insert at the head of Proportional and back of Monospace.
    ProportionalFront,
    /// Insert at the back of Proportional and Monospace (for fallback after
    /// glyph misses in earlier fonts).
    ProportionalBack,
}

fn try_install_font(
    fonts: &mut egui::FontDefinitions,
    key: &'static str,
    path: &str,
    slot: FontSlot,
) -> Option<&'static str> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(path, "could not load font: {err}");
            return None;
        }
    };
    fonts.font_data.insert(
        key.to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    match slot {
        FontSlot::ProportionalFront => {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, key.to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(key.to_owned());
        }
        FontSlot::ProportionalBack => {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push(key.to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(key.to_owned());
        }
    }
    Some(key)
}
