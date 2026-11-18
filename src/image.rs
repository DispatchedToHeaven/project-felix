//! PNG files the player brings (skins), decoded to RGBA.

/// `bytes` as RGBA8 pixels, with the width and height.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("too big")?];
    let info = r.next_frame(&mut buf).map_err(|e| format!("bad PNG: {e}"))?;
    let (w, h) = (info.width, info.height);
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colours were not expanded".into()),
    };
    Ok((w, h, rgba))
}

/// Deflate data inflated (zip entries made with deflate).
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    miniz_oxide::inflate::decompress_to_vec(data).map_err(|e| format!("bad deflate data: {e:?}"))
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}
