use eframe::egui::IconData;

/// Adds the visual margin expected by macOS while leaving other platforms'
/// icon artwork unchanged.
pub fn prepare_icon(icon: IconData) -> IconData {
    #[cfg(target_os = "macos")]
    return inset_macos_icon(icon);

    #[cfg(not(target_os = "macos"))]
    icon
}

#[cfg(target_os = "macos")]
fn inset_macos_icon(icon: IconData) -> IconData {
    use image::imageops::FilterType;

    let width = icon.width;
    let height = icon.height;
    let source = image::RgbaImage::from_raw(width, height, icon.rgba)
        .expect("decoded Scope2000 icon has valid RGBA dimensions");

    // Apple recommends keeping the main icon content within an approximately
    // 10% margin. The source artwork is full-bleed, so scale it to 80% and
    // center it on a transparent canvas of the original size.
    let inset_width = (source.width() * 4 / 5).max(1);
    let inset_height = (source.height() * 4 / 5).max(1);
    let resized = image::imageops::resize(&source, inset_width, inset_height, FilterType::Lanczos3);
    let offset_x = (source.width() - inset_width) / 2;
    let offset_y = (source.height() - inset_height) / 2;
    let mut canvas = image::RgbaImage::new(source.width(), source.height());
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from(offset_x),
        i64::from(offset_y),
    );

    IconData {
        rgba: canvas.into_raw(),
        width,
        height,
    }
}
