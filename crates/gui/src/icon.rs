//! The app icon, embedded in the binary and decoded once at startup.

use iced::widget::image;
use iced::window;

const ICON_256: &[u8] = include_bytes!("../../../assets/icons/png/256.png");
const ICON_64: &[u8] = include_bytes!("../../../assets/icons/png/64.png");

/// Decode one of our own (8-bit RGBA) icon PNGs into `(width, height, pixels)`.
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some((frame.width, frame.height, pixels))
}

/// Title-bar and taskbar icon (Windows, X11). Wayland and macOS take the icon from the
/// installed `.desktop` file or app bundle instead.
pub fn window_icon() -> Option<window::Icon> {
    let (width, height, pixels) = decode(ICON_256)?;
    window::icon::from_rgba(pixels, width, height).ok()
}

/// The logo shown in the sidebar. Create it once: every new handle is a new GPU texture.
pub fn logo() -> image::Handle {
    let (width, height, pixels) = decode(ICON_64).unwrap_or((1, 1, vec![0; 4]));
    image::Handle::from_rgba(width, height, pixels)
}
