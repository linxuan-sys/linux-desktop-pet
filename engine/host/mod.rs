// Wayland window host: present a composed pet frame in a borderless,
// transparent, always-on-top window. Input regions come from the opaque art,
// so clicks land on the pet only.

pub mod wl;

use crate::gfx::Canvas;

/// Which physical mouse button ended a stationary press. The Wayland host
/// normalizes raw codes (272/273/274) to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    /// Index into Config.click: [left, middle, right]
    pub fn index(self) -> usize {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        }
    }
}

/// Pointer/window events the host extracts from the display server.
#[derive(Debug, PartialEq)]
pub enum AppEvent {
    /// pointer press-release without movement; the "pet me" gesture
    Click(MouseButton),
    /// press-release after movement (compositor drag); pet position changed
    Dragged,
    GeometryChanged,
    Closed,
    Disconnected,
}

pub trait Host {
    /// False means both buffers are still owned by the compositor; retry later.
    fn present(&mut self, c: &Canvas) -> Result<bool, String>;
    fn set_scale(&mut self, scale: f64);
    fn placement(&self) -> crate::settings::Placement;
    fn dragging(&self) -> bool;
    /// buffer size in physical pixels (the canvas main renders into)
    fn size(&self) -> (u32, u32);
    /// move the pet window to a screen position (walking); bottom-anchored y
    fn move_to(&mut self, _x: i32, _y: i32) -> Result<(), String> {
        Ok(())
    }
    /// what move_to() considers the current window position
    fn position(&self) -> (i32, i32) {
        (0, 0)
    }
    /// widest legal window x (logical); 0 when the screen size is unknown
    fn max_x(&self) -> i32 {
        0
    }
    /// drain pointer/window events accumulated since the last call
    fn poll_events(&mut self) -> Vec<AppEvent>;
}

/// Open the Wayland window host. Call once from main.
pub fn open(settings: &crate::settings::Settings) -> Result<Box<dyn Host>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return Err("no Wayland session (WAYLAND_DISPLAY and WAYLAND_SOCKET unset)".into());
    }
    Ok(Box::new(
        wl::WlHost::new(settings).map_err(|e| format!("wayland host: {e}"))?,
    ))
}

/// wl_shm ARGB8888 little-endian wants B,G,R,A.
pub(crate) fn rgba_to_bgra(src: &[u8], dst: &mut Vec<u8>) {
    dst.resize(src.len(), 0);
    for (p, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        d.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
}

/// Opaque pixel rects (alpha > 32) in buffer pixels; the host maps them to
/// its input coordinate space. Empty means click-through. Row runs are merged
/// vertically when they line up, so a blob pet becomes 1–3 rects.
pub fn input_regions(c: &Canvas) -> Vec<(i32, i32, u32, u32)> {
    const ALPHA_MIN: u8 = 32;
    let mut rows: Vec<(i32, i32, u32)> = Vec::new(); // (y, x, len) runs per row
    for y in 0..c.height as i32 {
        let mut run_start: Option<i32> = None;
        for x in 0..c.width as i32 {
            let opaque = c.buf[((y as u32 * c.width + x as u32) * 4 + 3) as usize] > ALPHA_MIN;
            match (run_start, opaque) {
                (Some(_), true) => {}
                (Some(s), false) => {
                    rows.push((y, s, (x - s) as u32));
                    run_start = None;
                }
                (None, true) => run_start = Some(x),
                (None, false) => {}
            }
        }
        if let Some(s) = run_start {
            rows.push((y, s, (c.width as i32 - s) as u32));
        }
    }
    // Only the preceding row can extend a rectangle: avoid scanning every
    // historical rectangle for every run (quadratic on detailed silhouettes).
    let mut rects: Vec<(i32, i32, u32, u32)> = Vec::new();
    let mut active = std::collections::HashMap::<(i32, u32), usize>::new();
    let mut next = std::collections::HashMap::new();
    let mut row = -1;
    for (y, x, len) in rows {
        if y != row {
            std::mem::swap(&mut active, &mut next);
            next.clear();
            if y != row + 1 {
                active.clear();
            }
            row = y;
        }
        let index = if let Some(&index) = active.get(&(x, len)) {
            rects[index].3 += 1;
            index
        } else {
            rects.push((y, x, len, 1));
            rects.len() - 1
        };
        next.insert((x, len), index);
    }
    // return as (x, y, w, h) for host order
    rects.iter().map(|(y, x, w, h)| (*x, *y, *w, *h)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_region_keeps_holes_and_merges_only_touching_rows() {
        let mut canvas = Canvas::new(4, 3);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1), (3, 1), (3, 2)] {
            canvas.buf[(y * 4 + x) * 4 + 3] = 255;
        }
        assert_eq!(input_regions(&canvas), vec![(0, 0, 2, 2), (3, 1, 1, 2)]);
        canvas.buf.fill(0);
        assert!(input_regions(&canvas).is_empty());
    }
}
