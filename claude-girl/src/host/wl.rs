// Wayland host: the pet is a wlr-layer-shell OVERLAY surface — it floats
// above tiled windows, never takes focus, and never enters the taskbar.
// Position and drag live in the surface's anchored margins; pixels come
// from SHM buffers.
//
// shortcut: single-buffer present (commit while compositor may still read
// it; upgrade to double buffering if tearing shows).

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsFd;

use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_region, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use zwlr_layer_shell_v1::Layer;
use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};

use crate::gfx::{Canvas, WINDOW_H, WINDOW_W};
use super::{input_regions, rgba_to_bgra, AppEvent, Host, MouseButton};

/// Where the pet perches by default (upstream margin config).
const PERCH_MARGIN: i32 = 20;
/// The right side sits closer to the edge than the bottom.
const PERCH_MARGIN_RIGHT: i32 = 4;

/// The dispatch state: owns the protocol proxies and reacts to events.
struct Inner {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    viewport: Option<wp_viewport::WpViewport>,
    seat: Option<wl_seat::WlSeat>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    buffer: Option<wl_buffer::WlBuffer>,
    pool_file: Option<File>,
    events: Vec<AppEvent>,
    press_serial: u32,
    pointer_down: bool,
    pointer_moved: bool,
    pointer_prev: Option<(f64, f64)>,
    margin_right: i32,
    margin_bottom: i32,
    #[allow(dead_code)]
    pointer_holder: Option<wl_pointer::WlPointer>,
    #[allow(dead_code)]
    output_holder: Option<wl_output::WlOutput>,
    /// current output geometry from wl_output.mode (physical px)
    screen: (i32, i32),
    /// compositor's preferred scale ×120 (120 = 1.0), from fractional-scale
    preferred_scale: u32,
    configured: bool,
    qh: QueueHandle<Inner>,
}

impl Inner {
    /// Physical output size divided by the preferred scale: the logical
    /// coordinate space that layer-shell margins and window positions use.
    fn logical_screen(&self) -> (i32, i32) {
        let scale = self.preferred_scale.max(1) as i32;
        (self.screen.0 * 120 / scale, self.screen.1 * 120 / scale)
    }

    /// Surface-local size of the pet window: buffer px / output scale.
    fn logical_window(&self) -> (i32, i32) {
        if self.viewport.is_some() {
            let scale = self.preferred_scale.max(1) as i64;
            (
                ((WINDOW_W as i64 * 120 + scale / 2) / scale) as i32,
                ((WINDOW_H as i64 * 120 + scale / 2) / scale) as i32,
            )
        } else {
            // fallback without viewporter: integer buffer scale 2
            ((WINDOW_W / 2) as i32, (WINDOW_H / 2) as i32)
        }
    }

    /// Push the logical size to the layer surface and the viewport.
    fn apply_logical_size(&self) {
        let (w, h) = self.logical_window();
        if let Some(ls) = &self.layer_surface {
            ls.set_size(w.max(1) as u32, h.max(1) as u32);
        }
        if let Some(vp) = &self.viewport {
            vp.set_destination(w.max(1), h.max(1));
        }
    }
}

pub struct WlHost {
    conn: Connection,
    queue: EventQueue<Inner>,
    inner: Inner,
}

impl WlHost {
    pub fn new() -> Result<WlHost, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let mut inner = Inner {
            compositor: None,
            shm: None,
            layer_shell: None,
            fractional: None,
            viewporter: None,
            viewport: None,
            seat: None,
            surface: None,
            layer_surface: None,
            buffer: None,
            pool_file: None,
            events: Vec::new(),
            press_serial: 0,
            pointer_down: false,
            pointer_moved: false,
            pointer_prev: None,
            margin_right: PERCH_MARGIN_RIGHT,
            margin_bottom: PERCH_MARGIN,
            pointer_holder: None,
            output_holder: None,
            screen: (0, 0),
            preferred_scale: 120,
            configured: false,
            qh: qh.clone(),
        };
        conn.display().get_registry(&qh, ());
        roundtrip(&mut queue, &mut inner, "globals")?;
        // the wl_output bind above fires a Mode event; catch it before use
        roundtrip(&mut queue, &mut inner, "output modes")?;

        let compositor = inner.compositor.clone().ok_or("no wl_compositor global")?;
        let shm = inner.shm.clone().ok_or("no wl_shm global")?;
        let layer_shell = inner
            .layer_shell
            .clone()
            .ok_or("no zwlr_layer_shell_v1 (pet needs an overlay-capable compositor)")?;
        if inner.seat.is_none() {
            eprintln!("claude-girl: no wl_seat global; clicks/drag disabled on this compositor");
        }

        let surface = compositor.create_surface(&qh, ());
        if let Some(f) = inner.fractional.clone() {
            f.get_fractional_scale(&surface, &qh, ());
        }
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            Layer::Overlay,
            "claude-girl".to_owned(),
            &qh,
            (),
        );
        // bottom-right perch; -1 = overlap other layer surfaces, lets the
        // pet cross taskbars; None keyboard = never steals focus (chords
        // arrive via side channels instead)
        layer_surface.set_anchor(Anchor::Bottom | Anchor::Right);
        // provisional until the compositor reports its fractional scale
        layer_surface.set_size(WINDOW_W, WINDOW_H);
        // send the perch immediately: without this the surface starts at
        // margin 0, glued to the screen corner (art reads as screen-edge-cut)
        layer_surface.set_margin(0, PERCH_MARGIN_RIGHT, PERCH_MARGIN, 0);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        // viewporter maps the physical-pixel buffer onto the logical size
        if let Some(v) = inner.viewporter.clone() {
            inner.viewport = Some(v.get_viewport(&surface, &qh, ()));
        } else {
            // no viewporter: integer buffer scale keeps the 2x detail
            surface.set_buffer_scale(2);
        }

        // pool + fully transparent initial buffer (pet art goes in at present())
        let file = tempfile().map_err(|e| format!("tempfile: {e}"))?;
        file.set_len((WINDOW_W * WINDOW_H * 4) as u64)
            .map_err(|e| format!("set_len: {e}"))?;
        let size = (WINDOW_W * WINDOW_H * 4) as i32;
        let pool = shm.create_pool(file.as_fd(), size, &qh, ());
        let buffer = pool.create_buffer(
            0,
            WINDOW_W as i32,
            WINDOW_H as i32,
            (WINDOW_W * 4) as i32,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );
        inner.pool_file = Some(file);
        inner.buffer = Some(buffer);
        inner.surface = Some(surface);
        inner.layer_surface = Some(layer_surface);

        // layer-spec map dance: empty commit (no attach) requests the first
        // Configure; this commit also makes the compositor pick an output
        // and send the fractional preferred_scale
        if let Some(s) = &inner.surface {
            s.commit();
        }
        roundtrip(&mut queue, &mut inner, "layer configure")?;
        roundtrip(&mut queue, &mut inner, "preferred scale")?;

        // now that preferred_scale is known, apply the logical size and take
        // the second configure before any pet pixels are attached
        inner.apply_logical_size();
        if let Some(s) = &inner.surface {
            s.commit();
        }
        roundtrip(&mut queue, &mut inner, "layer reconfigure")?;

        Ok(WlHost { conn, queue, inner })
    }
}

fn roundtrip(queue: &mut EventQueue<Inner>, inner: &mut Inner, what: &str) -> Result<(), String> {
    queue
        .roundtrip(inner)
        .map(|_| ())
        .map_err(|e| format!("wayland {what}: {e}"))
}

fn commit_tree(inner: &Inner) {
    if let Some(s) = &inner.surface {
        if inner.configured {
            if let Some(buf) = &inner.buffer {
                s.attach(Some(buf), 0, 0);
                let (lw, lh) = inner.logical_window();
                s.damage(0, 0, lw.max(1), lh.max(1));
            }
        }
        s.commit();
    }
}

fn apply_input_region(inner: &Inner, canvas: &Canvas) {
    let Some(surface) = &inner.surface else { return };
    let Some(compositor) = &inner.compositor else { return };
    // canvas rects are buffer pixels; the input region is surface-local
    let (lw, _) = inner.logical_window();
    let ratio = WINDOW_W as f64 / lw.max(1) as f64;
    let region = compositor.create_region(&inner.qh, ());
    for (x, y, w, h) in input_regions(canvas) {
        let lx = (x as f64 / ratio).floor() as i32;
        let ly = (y as f64 / ratio).floor() as i32;
        let rw = ((w as f64 / ratio).ceil() as i32).max(1);
        let rh = ((h as f64 / ratio).ceil() as i32).max(1);
        region.add(lx, ly, rw, rh);
    }
    surface.set_input_region(Some(&region));
}

impl Host for WlHost {
    fn present(&mut self, c: &Canvas) {
        if let Some(f) = &mut self.inner.pool_file {
            let _ = f.seek(SeekFrom::Start(0));
            let _ = f.write_all(&rgba_to_bgra(&c.buf));
        }
        apply_input_region(&self.inner, c);
        commit_tree(&self.inner);
        // rs-buffered requests only leave on flush(); dispatch_pending does
        // NOT flush, and this host has no blocking dispatch in its loop
        let _ = self.conn.flush();
        let _ = self.queue.dispatch_pending(&mut self.inner);
    }

    fn size(&self) -> (u32, u32) {
        (WINDOW_W, WINDOW_H)
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        // bottom-right anchor: a logical screen position maps back into margins
        let (sw, sh) = self.inner.logical_screen();
        let (ww, wh) = self.inner.logical_window();
        self.inner.margin_right = (sw - x - ww).max(0);
        self.inner.margin_bottom = (sh - y - wh).max(0);
        if let Some(ls) = &self.inner.layer_surface {
            ls.set_margin(0, self.inner.margin_right, self.inner.margin_bottom, 0);
        }
        commit_tree(&self.inner);
        let _ = self.conn.flush();
        Ok(())
    }

    fn position(&self) -> (i32, i32) {
        let (sw, sh) = self.inner.logical_screen();
        if sw == 0 {
            return (0, 0);
        }
        let (ww, wh) = self.inner.logical_window();
        (
            sw - self.inner.margin_right - ww,
            sh - self.inner.margin_bottom - wh,
        )
    }

    fn max_x(&self) -> i32 {
        let (sw, _) = self.inner.logical_screen();
        let (ww, _) = self.inner.logical_window();
        (sw - ww).max(0)
    }

    fn poll_events(&mut self) -> Vec<AppEvent> {
        // dispatch_pending only drains events already read off the socket;
        // pointer events are read here, non-blocking (nothing else ever
        // reads between frames)
        if let Some(guard) = self.conn.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.inner);
        std::mem::take(&mut self.inner.events)
    }
}


impl Dispatch<wl_registry::WlRegistry, ()> for Inner {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ())),
                "wl_seat" => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ()))
                }
                "wl_output" => {
                    state.output_holder =
                        Some(registry.bind::<wl_output::WlOutput, _, _>(name, version.min(2), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind::<
                        zwlr_layer_shell_v1::ZwlrLayerShellV1,
                        _,
                        _,
                    >(name, version.min(4), qh, ()))
                }
                "wp_fractional_scale_manager_v1" => {
                    state.fractional = Some(registry.bind(name, version.min(1), qh, ()))
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Inner {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            if capabilities.contains(wl_seat::Capability::Pointer) {
                state.pointer_holder = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Inner {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_pointer::Event as E;
        match event {
            E::Motion { surface_x, surface_y, .. } => {
                let (px, py) = state
                    .pointer_prev
                    .replace((surface_x, surface_y))
                    .unwrap_or((surface_x, surface_y));
                if state.pointer_down {
                    let dx = (surface_x - px) as i32;
                    let dy = (surface_y - py) as i32;
                    if dx != 0 || dy != 0 {
                        state.pointer_moved = true;
                        // drag = slide the anchored margins: moving the
                        // pointer right/down slides the pet the same way
                        // (its surface-local coords reset as the surface
                        // moves with it, so tracking does not drift)
                        state.margin_right = (state.margin_right - dx).max(0);
                        state.margin_bottom = (state.margin_bottom - dy).max(0);
                        if let Some(ls) = &state.layer_surface {
                            ls.set_margin(0, state.margin_right, state.margin_bottom, 0);
                        }
                        // commit lands with the next present tick (~33ms)
                    }
                }
            }
            E::Button { button, state: btn, serial, .. } => {
                match btn {
                    WEnum::Value(wl_pointer::ButtonState::Pressed) => {
                        state.pointer_down = true;
                        state.pointer_moved = false;
                        state.pointer_prev = None;
                        state.press_serial = serial;
                    }
                    WEnum::Value(wl_pointer::ButtonState::Released) => {
                        state.pointer_down = false;
                        let ev = if state.pointer_moved {
                            AppEvent::Dragged
                        } else {
                            // button codes are the Linux input-event codes,
                            // pinned by the Wayland protocol (no constants in
                            // this wayland-client version — literals + comment)
                            AppEvent::Click(match button {
                                273 => MouseButton::Right,  // BTN_RIGHT
                                274 => MouseButton::Middle, // BTN_MIDDLE
                                _ => MouseButton::Left,     // BTN_LEFT & side buttons
                            })
                        };
                        state.events.push(ev);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Inner {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { flags, width, height, .. } = event {
            if matches!(flags, WEnum::Value(f) if f.contains(wl_output::Mode::Current)) {
                state.screen = (width as i32, height as i32);
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Inner {
    fn event(
        state: &mut Self,
        ls: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            ls.ack_configure(serial);
            state.configured = true;
            commit_tree(state);
        }
    }
}

delegate_noop!(Inner: ignore wl_surface::WlSurface);
delegate_noop!(Inner: ignore wl_buffer::WlBuffer);
delegate_noop!(Inner: ignore wl_shm::WlShm);
delegate_noop!(Inner: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Inner: ignore wl_region::WlRegion);
delegate_noop!(Inner: ignore wl_compositor::WlCompositor);
delegate_noop!(Inner: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(Inner: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Inner: ignore wp_viewporter::WpViewporter);
delegate_noop!(Inner: ignore wp_viewport::WpViewport);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Inner {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            if scale > 0 && scale != state.preferred_scale {
                state.preferred_scale = scale;
                state.apply_logical_size();
                if let Some(s) = &state.surface {
                    s.commit();
                }
            }
        }
    }
}
