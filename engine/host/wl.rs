//! Wayland owns geometry and buffer lifetimes; the renderer owns pixel content.
use super::{AppEvent, Host, MouseButton, input_regions, rgba_to_bgra};
use crate::gfx::{Canvas, WINDOW_H, WINDOW_W};
use crate::settings::{Placement, Settings};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsFd;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};

struct BufferSlot {
    buffer: wl_buffer::WlBuffer,
    file: File,
    size: (u32, u32),
    busy: bool,
}
struct Output {
    proxy: wl_output::WlOutput,
    name: String,
    mode: (i32, i32),
    rotated: bool,
    scale: i32,
}
struct Inner {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    viewport: Option<wp_viewport::WpViewport>,
    seat: Option<wl_seat::WlSeat>,
    surface: Option<wl_surface::WlSurface>,
    layer: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    buffers: Vec<BufferSlot>,
    bgra: Vec<u8>,
    events: Vec<AppEvent>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_down: bool,
    pointer_moved: bool,
    pointer_prev: Option<(f64, f64)>,
    right: i32,
    bottom: i32,
    scale: f64,
    preferred_scale: u32,
    has_fractional: bool,
    outputs: HashMap<u32, Output>,
    active_output: Option<u32>,
    configured: bool,
    geometry_dirty: bool,
    qh: QueueHandle<Inner>,
}

fn logical_window(scale: f64, output_scale: f64) -> (i32, i32) {
    // Divide by the compositor's real scale; it multiplies back by the same
    // number, so one buffer pixel lands on one output pixel.
    (
        (WINDOW_W as f64 * scale / output_scale).round().max(1.0) as i32,
        (WINDOW_H as f64 * scale / output_scale).round().max(1.0) as i32,
    )
}

fn motion_delta(previous: &mut Option<(f64, f64)>, current: (f64, f64)) -> (i32, i32) {
    let old = previous.replace(current).unwrap_or(current);
    (
        (current.0 - old.0).round() as i32,
        (current.1 - old.1).round() as i32,
    )
}
fn clamp_margins(right: i32, bottom: i32, screen: (i32, i32), window: (i32, i32)) -> (i32, i32) {
    (
        right.clamp(0, (screen.0 - window.0).max(0)),
        bottom.clamp(0, (screen.1 - window.1).max(0)),
    )
}
impl Inner {
    fn output_scale(&self) -> f64 {
        if self.has_fractional {
            self.preferred_scale.max(1) as f64 / 120.0
        } else {
            self.active_output
                .and_then(|id| self.outputs.get(&id))
                .map_or(1, |o| o.scale.max(1)) as f64
        }
    }
    fn logical_screen(&self) -> (i32, i32) {
        let Some(output) = self.active_output.and_then(|id| self.outputs.get(&id)) else {
            return (0, 0);
        };
        let (w, h) = if output.rotated {
            (output.mode.1, output.mode.0)
        } else {
            output.mode
        };
        let s = self.output_scale();
        ((w as f64 / s).floor() as i32, (h as f64 / s).floor() as i32)
    }
    fn logical_window(&self) -> (i32, i32) {
        logical_window(self.scale, self.output_scale())
    }
    /// The canvas is drawn at 652x372; at 100% the buffer matches it exactly,
    /// so `present` never resamples. Any resample re-interpolates the alpha
    /// edge every frame and shows up as speckle on the white areas.
    fn buffer_size(&self) -> (u32, u32) {
        (
            (WINDOW_W as f64 * self.scale).round().max(1.0) as u32,
            (WINDOW_H as f64 * self.scale).round().max(1.0) as u32,
        )
    }
    fn apply_geometry(&mut self) {
        let window = self.logical_window();
        let screen = self.logical_screen();
        if screen.0 > 0 && screen.1 > 0 {
            (self.right, self.bottom) = clamp_margins(self.right, self.bottom, screen, window);
        }
        if let Some(layer) = &self.layer {
            layer.set_size(window.0 as u32, window.1 as u32);
            layer.set_margin(0, self.right, self.bottom, 0);
        }
        if let Some(vp) = &self.viewport {
            vp.set_destination(window.0, window.1);
        } else if let Some(surface) = &self.surface {
            surface.set_buffer_scale(self.output_scale().ceil() as i32);
        }
        self.geometry_dirty = true;
    }
    fn commit_position(&self) {
        if let Some(s) = &self.surface {
            s.commit();
        }
    }
}

pub struct WlHost {
    conn: Connection,
    queue: EventQueue<Inner>,
    inner: Inner,
}
impl WlHost {
    pub fn new(settings: &Settings) -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("connect: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let env_margin = |key: &str, default| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(default)
                .max(0)
        };
        let placement = Placement::load().unwrap_or(Placement {
            right: env_margin("PET_MARGIN_RIGHT", 4),
            bottom: env_margin("PET_MARGIN_BOTTOM", 20),
        });
        let mut inner = Inner {
            compositor: None,
            shm: None,
            layer_shell: None,
            fractional: None,
            viewporter: None,
            viewport: None,
            seat: None,
            surface: None,
            layer: None,
            buffers: Vec::new(),
            bgra: Vec::new(),
            events: Vec::new(),
            pointer: None,
            pointer_down: false,
            pointer_moved: false,
            pointer_prev: None,
            right: placement.right,
            bottom: placement.bottom,
            scale: settings.scale,
            preferred_scale: 120,
            has_fractional: false,
            outputs: HashMap::new(),
            active_output: None,
            configured: false,
            geometry_dirty: true,
            qh: qh.clone(),
        };
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut inner).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut inner).map_err(|e| e.to_string())?;
        let compositor = inner.compositor.clone().ok_or("no wl_compositor")?;
        let layer_shell = inner
            .layer_shell
            .clone()
            .ok_or("compositor does not support layer-shell")?;
        if inner.shm.is_none() {
            return Err("no wl_shm".into());
        }
        let chosen = settings.output.as_ref().and_then(|name| {
            inner
                .outputs
                .iter()
                .find(|(_, o)| &o.name == name)
                .map(|(id, o)| (*id, o.proxy.clone()))
        });
        if settings.output.is_some() && chosen.is_none() {
            eprintln!("configured output unavailable; using compositor-selected output");
        }
        inner.active_output = chosen.as_ref().map(|(id, _)| *id);
        let surface = compositor.create_surface(&qh, ());
        if let Some(f) = &inner.fractional {
            f.get_fractional_scale(&surface, &qh, ());
        }
        if let Some(v) = &inner.viewporter {
            inner.viewport = Some(v.get_viewport(&surface, &qh, ()));
        }
        let layer = layer_shell.get_layer_surface(
            &surface,
            chosen.as_ref().map(|(_, o)| o),
            zwlr_layer_shell_v1::Layer::Overlay,
            env!("CARGO_PKG_NAME").into(),
            &qh,
            (),
        );
        layer.set_anchor(Anchor::Bottom | Anchor::Right);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        inner.surface = Some(surface);
        inner.layer = Some(layer);
        inner.apply_geometry();
        inner.commit_position();
        queue.roundtrip(&mut inner).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut inner).map_err(|e| e.to_string())?;
        inner.apply_geometry();
        inner.commit_position();
        queue.roundtrip(&mut inner).map_err(|e| e.to_string())?;
        Ok(Self { conn, queue, inner })
    }
    fn free_slot(&mut self, size: (u32, u32)) -> Result<Option<usize>, String> {
        let i = match self.inner.buffers.iter().position(|b| !b.busy) {
            Some(i) => i,
            None if self.inner.buffers.len() < 2 => self.inner.buffers.len(),
            None => return Ok(None),
        };
        if i < self.inner.buffers.len() && self.inner.buffers[i].size == size {
            return Ok(Some(i));
        }
        let (w, h) = size;
        let file = tempfile::tempfile().map_err(|e| e.to_string())?;
        let len = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .ok_or("buffer dimensions overflow")?;
        file.set_len(len as u64).map_err(|e| e.to_string())?;
        let pool = self.inner.shm.as_ref().unwrap().create_pool(
            file.as_fd(),
            len as i32,
            &self.inner.qh,
            (),
        );
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            (w * 4) as i32,
            wl_shm::Format::Argb8888,
            &self.inner.qh,
            (),
        );
        pool.destroy(); // buffer keeps the pool's backing store alive
        let slot = BufferSlot {
            buffer,
            file,
            size,
            busy: false,
        };
        if i < self.inner.buffers.len() {
            self.inner.buffers[i].buffer.destroy();
            self.inner.buffers[i] = slot;
        } else {
            self.inner.buffers.push(slot);
        }
        Ok(Some(i))
    }
}
fn apply_input_region(inner: &Inner, canvas: &Canvas) {
    let surface = inner.surface.as_ref().unwrap();
    let region = inner
        .compositor
        .as_ref()
        .unwrap()
        .create_region(&inner.qh, ());
    let (lw, lh) = inner.logical_window();
    let (rx, ry) = (
        lw as f64 / canvas.width as f64,
        lh as f64 / canvas.height as f64,
    );
    for (x, y, w, h) in input_regions(canvas) {
        let left = (x as f64 * rx).floor() as i32;
        let top = (y as f64 * ry).floor() as i32;
        let right = ((x as f64 + w as f64) * rx).ceil() as i32;
        let bottom = ((y as f64 + h as f64) * ry).ceil() as i32;
        region.add(left, top, right - left, bottom - top);
    }
    surface.set_input_region(Some(&region));
    region.destroy();
}
impl Host for WlHost {
    fn present(&mut self, canvas: &Canvas) -> Result<bool, String> {
        if !self.inner.configured {
            return Ok(false);
        }
        let size = self.size();
        let Some(i) = self.free_slot(size)? else {
            return Ok(false);
        };
        let resized;
        let c = if (canvas.width, canvas.height) == size {
            canvas
        } else {
            resized = canvas.resize(size.0, size.1);
            &resized
        };
        rgba_to_bgra(&c.buf, &mut self.inner.bgra);
        let slot = &mut self.inner.buffers[i];
        slot.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        slot.file
            .write_all(&self.inner.bgra)
            .map_err(|e| e.to_string())?;
        apply_input_region(&self.inner, c);
        let slot = &mut self.inner.buffers[i];
        slot.busy = true;
        let surface = self.inner.surface.as_ref().unwrap();
        surface.attach(Some(&slot.buffer), 0, 0);
        // Buffer coordinates: with a viewport the surface is smaller than the
        // buffer, and surface damage would leave the rest stale.
        surface.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        surface.commit();
        self.inner.geometry_dirty = false;
        self.conn.flush().map_err(|e| e.to_string())?;
        Ok(true)
    }
    fn size(&self) -> (u32, u32) {
        self.inner.buffer_size()
    }
    fn set_scale(&mut self, scale: f64) {
        let old = self.inner.logical_window();
        self.inner.scale = scale;
        let new = self.inner.logical_window();
        self.inner.right += (old.0 - new.0) / 2;
        self.inner.apply_geometry(); // bottom anchor is stable; horizontal center stays put
    }
    fn placement(&self) -> Placement {
        Placement {
            right: self.inner.right,
            bottom: self.inner.bottom,
        }
    }
    fn dragging(&self) -> bool {
        self.inner.pointer_down
    }
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        let (sw, sh) = self.inner.logical_screen();
        let (ww, wh) = self.inner.logical_window();
        let next = clamp_margins(sw - x - ww, sh - y - wh, (sw, sh), (ww, wh));
        if next == (self.inner.right, self.inner.bottom) {
            return Ok(());
        }
        (self.inner.right, self.inner.bottom) = next;
        self.inner.apply_geometry();
        self.inner.commit_position();
        self.conn.flush().map_err(|e| e.to_string())
    }
    fn position(&self) -> (i32, i32) {
        let (sw, sh) = self.inner.logical_screen();
        let (ww, wh) = self.inner.logical_window();
        (
            (sw - ww - self.inner.right).max(0),
            (sh - wh - self.inner.bottom).max(0),
        )
    }
    fn max_x(&self) -> i32 {
        (self.inner.logical_screen().0 - self.inner.logical_window().0).max(0)
    }
    fn poll_events(&mut self) -> Vec<AppEvent> {
        if let Some(guard) = self.conn.prepare_read()
            && let Err(e) = guard.read()
            && !matches!(&e, wayland_client::backend::WaylandError::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock)
        {
            eprintln!("Wayland read: {e}");
            self.inner.events.push(AppEvent::Disconnected);
        }
        if let Err(e) = self.queue.dispatch_pending(&mut self.inner) {
            eprintln!("Wayland dispatch: {e}");
            self.inner.events.push(AppEvent::Disconnected);
        }
        if self.inner.geometry_dirty {
            self.inner.events.push(AppEvent::GeometryChanged);
        }
        std::mem::take(&mut self.inner.events)
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for Inner {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => s.compositor = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_shm" => s.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" => s.seat = Some(registry.bind(name, 1, qh, ())),
                "wl_output" => {
                    let proxy = registry.bind(name, version.min(4), qh, name);
                    s.outputs.insert(
                        name,
                        Output {
                            proxy,
                            name: String::new(),
                            mode: (0, 0),
                            rotated: false,
                            scale: 1,
                        },
                    );
                }
                "zwlr_layer_shell_v1" => {
                    s.layer_shell = Some(registry.bind(name, version.min(4), qh, ()))
                }
                "wp_fractional_scale_manager_v1" => {
                    s.fractional = Some(registry.bind(name, 1, qh, ()))
                }
                "wp_viewporter" => s.viewporter = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(o) = s.outputs.remove(&name)
                    && o.proxy.version() >= 3
                {
                    o.proxy.release();
                }
                if s.active_output == Some(name) {
                    s.active_output = None;
                    s.events.push(AppEvent::Closed);
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for Inner {
    fn event(
        s: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = s.outputs.get_mut(id) else {
            return;
        };
        match event {
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => o.mode = (width, height),
            wl_output::Event::Name { name } => o.name = name,
            wl_output::Event::Scale { factor } => o.scale = factor.max(1),
            wl_output::Event::Geometry {
                transform: WEnum::Value(t),
                ..
            } => {
                o.rotated = matches!(
                    t,
                    wl_output::Transform::_90
                        | wl_output::Transform::_270
                        | wl_output::Transform::Flipped90
                        | wl_output::Transform::Flipped270
                );
            }
            _ => {}
        }
        if s.active_output == Some(*id) {
            s.apply_geometry();
        }
    }
}
impl Dispatch<wl_surface::WlSurface, ()> for Inner {
    fn event(
        s: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_surface::Event::Enter { output } = event {
            s.active_output = s
                .outputs
                .iter()
                .find(|(_, o)| o.proxy == output)
                .map(|(id, _)| *id);
            s.apply_geometry();
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, ()> for Inner {
    fn event(
        s: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event
            && let Some(slot) = s.buffers.iter_mut().find(|slot| &slot.buffer == buffer)
        {
            slot.busy = false;
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for Inner {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(c),
        } = event
            && c.contains(wl_seat::Capability::Pointer)
            && s.pointer.is_none()
        {
            s.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Inner {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                // Consume each motion delta once, including compositor-generated
                // duplicate motions during an implicit pointer grab.
                let (dx, dy) = motion_delta(&mut s.pointer_prev, (surface_x, surface_y));
                if s.pointer_down && (dx != 0 || dy != 0) {
                    s.pointer_moved = true;
                    s.right -= dx;
                    s.bottom -= dy;
                    s.apply_geometry();
                    s.commit_position();
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(state),
                ..
            } if [272, 273, 274].contains(&button) => {
                if state == wl_pointer::ButtonState::Pressed {
                    s.pointer_down = true;
                    s.pointer_moved = false;
                } else if s.pointer_down {
                    s.pointer_down = false;
                    s.events.push(if s.pointer_moved {
                        AppEvent::Dragged
                    } else {
                        AppEvent::Click(match button {
                            273 => MouseButton::Right,
                            274 => MouseButton::Middle,
                            _ => MouseButton::Left,
                        })
                    });
                }
            }
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => s.pointer_prev = Some((surface_x, surface_y)),
            wl_pointer::Event::Leave { .. } if !s.pointer_down => s.pointer_prev = None,
            _ => {}
        }
    }
}
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Inner {
    fn event(
        s: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
                s.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => s.events.push(AppEvent::Closed),
            _ => {}
        }
    }
}
impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Inner {
    fn event(
        s: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && scale > 0
            && (s.preferred_scale != scale || !s.has_fractional)
        {
            s.preferred_scale = scale;
            s.has_fractional = true;
            s.apply_geometry();
        }
    }
}
delegate_noop!(Inner: ignore wl_shm::WlShm);
delegate_noop!(Inner: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Inner: ignore wl_region::WlRegion);
delegate_noop!(Inner: ignore wl_compositor::WlCompositor);
delegate_noop!(Inner: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(Inner: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Inner: ignore wp_viewporter::WpViewporter);
delegate_noop!(Inner: ignore wp_viewport::WpViewport);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_pointer_events_do_not_multiply_drag_distance() {
        let mut previous = Some((200.0, 100.0));
        let mut total = (0, 0);
        for i in 1..=10 {
            let point = (200.0 - 12.0 * i as f64, 100.0 - 6.0 * i as f64);
            let delta = motion_delta(&mut previous, point);
            total.0 += delta.0;
            total.1 += delta.1;
            assert_eq!(motion_delta(&mut previous, point), (0, 0));
        }
        assert_eq!(total, (-120, -60));
    }
    #[test]
    fn size_is_logical_and_does_not_depend_on_monitor_dpi() {
        assert_eq!(logical_window(1.0, 1.75), (373, 213));
        assert_eq!(logical_window(0.5, 1.75), (186, 106));
        assert_eq!(logical_window(1.0, 1.5), (435, 248));
    }
    #[test]
    fn restore_outside_smaller_monitor_stays_on_screen() {
        assert_eq!(
            clamp_margins(2000, 900, (1280, 720), (435, 248)),
            (845, 472)
        );
        assert_eq!(clamp_margins(-100, -20, (1280, 720), (435, 248)), (0, 0));
    }
}
