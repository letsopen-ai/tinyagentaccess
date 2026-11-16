use std::io::Cursor;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Debug)]
pub struct CursorImage {
    pub position: Point,
    pub hotspot: Point,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub logical_size: (u32, u32),
    pub rgba: Vec<u8>,
    pub cursor: Option<CursorImage>,
}
impl Frame {
    pub fn solid(width: u32, height: u32, color: [u8; 4]) -> Self {
        Self {
            width,
            height,
            logical_size: (width, height),
            rgba: color.repeat(width as usize * height as usize),
            cursor: None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 32768
            || self.height > 32768
            || self.rgba.len() != self.width as usize * self.height as usize * 4
        {
            Err("invalid desktop frame dimensions".into())
        } else {
            Ok(())
        }
    }
}
/// Rounded integer viewport, shared by rendering and inverse mapping.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub native: (u32, u32),
    pub agent: (u32, u32),
    pub viewport: (u32, u32, u32, u32),
}
impl Mapping {
    pub fn new(native: (u32, u32), native_mode: bool) -> Self {
        let (w, h) = (native.0.max(1), native.1.max(1));
        let agent = if native_mode { (w, h) } else { (1280, 720) };
        let scale = (agent.0 as f64 / w as f64).min(agent.1 as f64 / h as f64);
        let vw = (w as f64 * scale).round().max(1.) as u32;
        let vh = (h as f64 * scale).round().max(1.) as u32;
        Self {
            native: (w, h),
            agent,
            viewport: ((agent.0 - vw) / 2, (agent.1 - vh) / 2, vw, vh),
        }
    }
    /// Endpoint-aligned nearest integer; bars clamp to the nearest physical edge.
    pub fn inverse(&self, p: Point) -> Point {
        let (x, y, w, h) = self.viewport;
        Point {
            x: map_axis(
                (p.x - x as i32).clamp(0, w as i32 - 1) as u32,
                w,
                self.native.0,
            ),
            y: map_axis(
                (p.y - y as i32).clamp(0, h as i32 - 1) as u32,
                h,
                self.native.1,
            ),
        }
    }
    pub fn forward(&self, p: Point) -> Point {
        let (x, y, w, h) = self.viewport;
        Point {
            x: x as i32 + map_axis(p.x.max(0) as u32, self.native.0, w),
            y: y as i32 + map_axis(p.y.max(0) as u32, self.native.1, h),
        }
    }
}
fn map_axis(v: u32, from: u32, to: u32) -> i32 {
    if from <= 1 {
        0
    } else {
        (v.min(from - 1) as f64 * (to - 1) as f64 / (from - 1) as f64).round() as i32
    }
}
fn draw_cursor(f: &mut Frame) {
    if let Some(c) = &f.cursor {
        for y in 0..c.height {
            for x in 0..c.width {
                let dx = c.position.x - c.hotspot.x + x as i32;
                let dy = c.position.y - c.hotspot.y + y as i32;
                if dx < 0 || dy < 0 || dx >= f.width as i32 || dy >= f.height as i32 {
                    continue;
                }
                let si = (y as usize * c.width as usize + x as usize) * 4;
                if si + 4 > c.rgba.len() {
                    continue;
                }
                let di = (dy as usize * f.width as usize + dx as usize) * 4;
                let a = c.rgba[si + 3] as u32;
                for k in 0..3 {
                    f.rgba[di + k] = ((c.rgba[si + k] as u32 * a
                        + f.rgba[di + k] as u32 * (255 - a))
                        / 255) as u8;
                }
            }
        }
    }
}
pub fn arrow(position: Point) -> CursorImage {
    let mut rgba = vec![0; 12 * 18 * 4];
    for y in 0..18 {
        for x in 0..12 {
            if x <= y / 2 {
                let i = (y * 12 + x) * 4;
                let color = if x == 0 || x == y / 2 { 0 } else { 255 };
                rgba[i..i + 4].copy_from_slice(&[color, color, color, 255]);
            }
        }
    }
    CursorImage {
        position,
        hotspot: Point { x: 0, y: 0 },
        width: 12,
        height: 18,
        rgba,
    }
}
pub fn render(mut frame: Frame, native: bool, cursor: bool) -> Result<Frame, String> {
    frame.validate()?;
    if cursor {
        draw_cursor(&mut frame);
    }
    if native {
        return Ok(frame);
    }
    let m = Mapping::new((frame.width, frame.height), native);
    let mut out = Frame::solid(m.agent.0, m.agent.1, [0, 0, 0, 255]);
    let (ox, oy, w, h) = m.viewport;
    // Bilinear interpolation, endpoint aligned to the exact input mapping.
    for y in 0..h {
        let sy = if h <= 1 {
            0.
        } else {
            y as f64 * (frame.height - 1) as f64 / (h - 1) as f64
        };
        let y0 = sy as u32;
        let fy = sy - y0 as f64;
        for x in 0..w {
            let sx = if w <= 1 {
                0.
            } else {
                x as f64 * (frame.width - 1) as f64 / (w - 1) as f64
            };
            let x0 = sx as u32;
            let fx = sx - x0 as f64;
            let di = (((y + oy) * out.width + x + ox) * 4) as usize;
            for k in 0..3 {
                let sample = |xx: u32, yy: u32| {
                    frame.rgba[((yy * frame.width + xx) * 4) as usize + k] as f64
                };
                let a = sample(x0, y0) * (1. - fx) + sample((x0 + 1).min(frame.width - 1), y0) * fx;
                let b = sample(x0, (y0 + 1).min(frame.height - 1)) * (1. - fx)
                    + sample(
                        (x0 + 1).min(frame.width - 1),
                        (y0 + 1).min(frame.height - 1),
                    ) * fx;
                out.rgba[di + k] = (a * (1. - fy) + b * fy).round() as u8;
            }
        }
    }
    Ok(out)
}
pub fn encode(frame: &Frame, jpeg: bool) -> Result<(Vec<u8>, &'static str), String> {
    frame.validate()?;
    let mut bytes = vec![];
    if jpeg {
        let rgb: Vec<_> = frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| p[..3].iter().copied())
            .collect();
        let w = u16::try_from(frame.width).map_err(|e| e.to_string())?;
        let h = u16::try_from(frame.height).map_err(|e| e.to_string())?;
        jpeg_encoder::Encoder::new(&mut bytes, 85)
            .encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| e.to_string())?;
        Ok((bytes, "image/jpeg"))
    } else {
        let mut enc = png::Encoder::new(&mut bytes, frame.width, frame.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .map_err(|e| e.to_string())?
            .write_image_data(&frame.rgba)
            .map_err(|e| e.to_string())?;
        Ok((bytes, "image/png"))
    }
}
fn png_error(error: png::DecodingError) -> String {
    match error {
        png::DecodingError::LimitsExceeded => {
            "portal PNG exceeds the 256 MiB decoder resource limit".into()
        }
        other => format!("portal PNG decode failed: {other}"),
    }
}
pub fn decode_png(bytes: &[u8]) -> Result<Frame, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: 256 * 1024 * 1024,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = decoder.read_info().map_err(png_error)?;
    if u64::from(r.info().width) * u64::from(r.info().height) > 64 * 1024 * 1024 {
        return Err("portal PNG exceeds 64 megapixel decoded image limit".into());
    }
    let mut data = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut data).map_err(png_error)?;
    let mut f = Frame::solid(info.width, info.height, [0, 0, 0, 255]);
    let channels = info.color_type.samples();
    for (p, q) in data[..info.buffer_size()]
        .chunks_exact(channels)
        .zip(f.rgba.as_chunks_mut::<4>().0.iter_mut())
    {
        match info.color_type {
            png::ColorType::Rgba => q.copy_from_slice(p),
            png::ColorType::Rgb => q[..3].copy_from_slice(p),
            png::ColorType::Grayscale => q[..3].fill(p[0]),
            png::ColorType::GrayscaleAlpha => {
                q[..3].fill(p[0]);
                q[3] = p[1];
            }
            _ => return Err("unsupported portal PNG color".into()),
        }
    }
    Ok(f)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mappings() {
        for size in [
            (1920, 1080),
            (1920, 1200),
            (1600, 1200),
            (3440, 1440),
            (1080, 1920),
            (2560, 1440),
            (2400, 1350),
        ] {
            let m = Mapping::new(size, false);
            for p in [
                Point { x: 0, y: 0 },
                Point { x: 1279, y: 719 },
                Point { x: 640, y: 360 },
            ] {
                let n = m.inverse(p);
                assert!(n.x >= 0 && n.x < size.0 as i32 && n.y >= 0 && n.y < size.1 as i32);
                let back = m.forward(n);
                let (x, y, w, h) = m.viewport;
                assert_eq!(back.x, p.x.clamp(x as i32, (x + w - 1) as i32));
                assert_eq!(back.y, p.y.clamp(y as i32, (y + h - 1) as i32));
            }
            let m = Mapping::new(size, true);
            assert_eq!(
                m.inverse(Point { x: 100, y: 100 }),
                Point { x: 100, y: 100 }
            );
        }
    }
    #[test]
    fn images() {
        assert!(png_error(png::DecodingError::LimitsExceeded)
            .contains("256 MiB decoder resource limit"));
        let mut f = Frame::solid(40, 30, [255, 0, 0, 255]);
        f.cursor = Some(arrow(Point { x: 39, y: 29 }));
        let out = render(f, false, true).unwrap();
        assert_eq!((out.width, out.height), (1280, 720));
        assert_eq!(&out.rgba[..4], &[0, 0, 0, 255]);
        for jpeg in [false, true] {
            let (b, _) = encode(&out, jpeg).unwrap();
            if jpeg {
                let mut d = jpeg_decoder::Decoder::new(Cursor::new(b));
                d.decode().unwrap();
                assert_eq!(d.info().unwrap().width, 1280);
            } else {
                let d = decode_png(&b).unwrap();
                assert_eq!(d.rgba, out.rgba);
            }
        }
    }
}

/// Current primary-display pixels to global logical coordinates, including its origin.
pub fn desktop_point(
    p: Point,
    physical: (u32, u32),
    logical: (f64, f64),
    origin: (f64, f64),
) -> (f64, f64) {
    (
        origin.0 + p.x as f64 * logical.0 / physical.0.max(1) as f64,
        origin.1 + p.y as f64 * logical.1 / physical.1.max(1) as f64,
    )
}
/// Physical capture pixels to OS logical input units; preserves fractional positions.
pub fn logical_point(p: Point, physical: (u32, u32), logical: (u32, u32)) -> (f64, f64) {
    desktop_point(p, physical, (logical.0 as f64, logical.1 as f64), (0., 0.))
}
#[cfg(test)]
mod dpi_tests {
    use super::*;
    #[test]
    fn scales() {
        assert_eq!(
            desktop_point(
                Point { x: 2879, y: 1799 },
                (2880, 1800),
                (1440., 900.),
                (-1440., 100.)
            ),
            (-0.5, 999.5)
        );
        let edge = desktop_point(
            Point { x: 2000, y: 1000 },
            (2001, 1001),
            (1600.8, 800.8),
            (50., -800.8),
        );
        assert!((edge.0 - 1650.).abs() < 1e-9 && (edge.1 + 0.8).abs() < 1e-9);
        assert_eq!(
            desktop_point(
                Point { x: 0, y: 0 },
                (2001, 1001),
                (1600.8, 800.8),
                (50., -800.8)
            ),
            (50., -800.8)
        );
        // The same physical point uses the refreshed mode and primary origin.
        assert_eq!(
            desktop_point(
                Point { x: 1200, y: 600 },
                (2400, 1200),
                (1200., 600.),
                (0., 0.)
            ),
            (600., 300.)
        );
        assert_eq!(
            desktop_point(
                Point { x: 1200, y: 600 },
                (2400, 1200),
                (2400., 1200.),
                (-2400., 0.)
            ),
            (-1200., 600.)
        );
        for (physical, logical) in [
            ((2560, 1440), (1280, 720)),
            ((1600, 900), (1280, 720)),
            ((1920, 1080), (1280, 720)),
        ] {
            let p = logical_point(
                Point {
                    x: physical.0 as i32 / 2,
                    y: physical.1 as i32 / 2,
                },
                physical,
                logical,
            );
            assert_eq!(p, (640., 360.));
        }
        let m = Mapping::new((1280, 800), false);
        assert_eq!(m.viewport, (64, 0, 1152, 720));
        assert_eq!(m.inverse(Point { x: 0, y: 719 }), Point { x: 0, y: 799 });
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    #[test]
    fn inclusion_and_clipping() {
        for position in [
            Point { x: -4, y: -4 },
            Point { x: 0, y: 0 },
            Point { x: 7, y: 7 },
            Point { x: 100, y: 100 },
        ] {
            let mut f = Frame::solid(8, 8, [200, 100, 50, 255]);
            f.cursor = Some(arrow(position));
            let off = render(f.clone(), true, false).unwrap();
            assert_eq!(off.rgba, [200, 100, 50, 255].repeat(64));
            let on = render(f, true, true).unwrap();
            assert_eq!(on.rgba.len(), 256);
            if position.x == 0 {
                assert_eq!(&on.rgba[..4], &[0, 0, 0, 255]);
            }
            if position.x == 100 {
                assert_eq!(on.rgba, off.rgba);
            }
        }
    }
}
