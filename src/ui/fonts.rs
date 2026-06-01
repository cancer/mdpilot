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
pub(crate) enum FontSlot {
    /// Insert at the head of Proportional and back of Monospace.
    ProportionalFront,
    /// Insert at the back of Proportional and Monospace (for fallback after
    /// glyph misses in earlier fonts).
    ProportionalBack,
}

pub(crate) fn try_install_font(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_yields_none_without_touching_definitions() {
        let mut fonts = egui::FontDefinitions::default();
        let baseline_keys: Vec<String> = fonts.font_data.keys().cloned().collect();

        let result = try_install_font(
            &mut fonts,
            "ghost",
            "/definitely/does/not/exist.ttf",
            FontSlot::ProportionalFront,
        );

        assert!(result.is_none());
        assert!(!fonts.font_data.contains_key("ghost"));
        let after_keys: Vec<String> = fonts.font_data.keys().cloned().collect();
        assert_eq!(baseline_keys, after_keys);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn aqua_kana_loads_at_front_of_proportional() {
        let mut fonts = egui::FontDefinitions::default();
        let result = try_install_font(
            &mut fonts,
            "test_aqua",
            "/System/Library/Fonts/AquaKana.ttc",
            FontSlot::ProportionalFront,
        );
        assert_eq!(result, Some("test_aqua"));
        assert!(fonts.font_data.contains_key("test_aqua"));
        let proportional = fonts.families.get(&egui::FontFamily::Proportional).unwrap();
        assert_eq!(proportional.first().map(String::as_str), Some("test_aqua"));
        let monospace = fonts.families.get(&egui::FontFamily::Monospace).unwrap();
        assert_eq!(monospace.last().map(String::as_str), Some("test_aqua"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn proportional_back_appends_rather_than_prepends() {
        let mut fonts = egui::FontDefinitions::default();
        // First insert at the front so we have a known head.
        let _ = try_install_font(
            &mut fonts,
            "front",
            "/System/Library/Fonts/AquaKana.ttc",
            FontSlot::ProportionalFront,
        );
        // Then insert at the back; head should be unchanged.
        let result = try_install_font(
            &mut fonts,
            "back",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            FontSlot::ProportionalBack,
        );
        assert_eq!(result, Some("back"));
        let proportional = fonts.families.get(&egui::FontFamily::Proportional).unwrap();
        assert_eq!(proportional.first().map(String::as_str), Some("front"));
        assert_eq!(proportional.last().map(String::as_str), Some("back"));
    }
}
