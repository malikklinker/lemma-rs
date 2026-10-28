//! CPU font coverage/extent validation shared by compilation and GPU preparation.
use anyhow::{Result, ensure};
use fontdue::{
    Font,
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

pub(crate) fn font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(
            include_bytes!("../assets/DejaVuSans.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .expect("the bundled DejaVu font must be valid")
    })
}

pub(crate) fn layout(text: &str, size: f32) -> Result<(Layout, [u32; 2])> {
    ensure!(
        size.is_finite() && size > 0.0 && size <= 8192.0,
        "resolved font size must be finite and in (0, 8192]"
    );
    for c in text.chars() {
        ensure!(
            font().lookup_glyph_index(c) != 0,
            "bundled font does not contain character {c:?}"
        );
    }
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: 1.0,
        y: 1.0,
        ..Default::default()
    });
    layout.append(&[font()], &TextStyle::new(text, size, 0));
    let width = layout
        .glyphs()
        .iter()
        .map(|g| g.x + g.width as f32)
        .fold(1.0_f32, f32::max)
        .ceil() as u32
        + 2;
    let height = layout
        .glyphs()
        .iter()
        .map(|g| g.y + g.height as f32)
        .fold(layout.height(), f32::max)
        .ceil() as u32
        + 2;
    ensure!(
        width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216,
        "text texture {width}x{height} exceeds the supported extent; split the text into smaller runs"
    );
    Ok((layout, [width, height]))
}

pub(crate) fn extent(text: &str, size: f32) -> Result<[u32; 2]> {
    // A bounded CPU metadata cache avoids reshaping unchanged text per sample.
    type CachedExtent = (String, u32, [u32; 2]);
    static VALIDATED: OnceLock<Mutex<VecDeque<CachedExtent>>> = OnceLock::new();
    let cache = VALIDATED.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Some((_, _, extent)) = cache
        .lock()
        .unwrap()
        .iter()
        .find(|(s, n, _)| s == text && *n == size.to_bits())
    {
        return Ok(*extent);
    }
    let (_, extent) = layout(text, size)?;
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 256 {
        cache.pop_front();
    }
    cache.push_back((text.into(), size.to_bits(), extent));
    Ok(extent)
}

pub(crate) fn validate(text: &str, size: f32) -> Result<()> {
    extent(text, size).map(|_| ())
}
