//! Wayland backend: a wlr layer-shell surface on the `overlay` layer, anchored to every edge of
//! one output, no keyboard focus and an empty input region (clicks pass through), drawn into an
//! shm buffer. It redraws only when the state changed, at most once per frame callback (so never
//! faster than the monitor refreshes).

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Event};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    registry_handlers,
};
use tiny_skia::Pixmap;

use crate::model::{Draw, Msg};
use crate::render::{render, to_bgra};

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    _region: Option<Region>,
    draw: Draw,
    pm: Option<Pixmap>,
    /// Logical surface size from the compositor, and the buffer scale.
    size: (u32, u32),
    scale: i32,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    exit: bool,
}

impl App {
    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        if !self.configured || !self.dirty || self.frame_pending {
            return;
        }
        let Some(layer) = &self.layer else { return };
        let (w, h) = (
            self.size.0 * self.scale as u32,
            self.size.1 * self.scale as u32,
        );
        if self
            .pm
            .as_ref()
            .is_none_or(|p| (p.width(), p.height()) != (w, h))
        {
            self.pm = Pixmap::new(w, h);
        }
        let Some(pm) = &mut self.pm else { return };
        render(&self.draw, pm);
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        to_bgra(pm, canvas);
        let s = layer.wl_surface();
        let _ = layer.set_buffer_scale(self.scale as u32);
        s.damage_buffer(0, 0, w as i32, h as i32);
        s.frame(qh, s.clone());
        if buffer.attach_to(s).is_ok() {
            layer.commit();
            self.frame_pending = true;
            self.dirty = false;
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.scale = new_factor.max(1);
        self.dirty = true;
        self.redraw(qh);
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
        self.redraw(qh);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if w == 0 || h == 0 {
            return;
        }
        self.size = (w, h);
        self.configured = true;
        self.dirty = true;
        self.redraw(qh);
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_layer!(App);
delegate_registry!(App);

pub fn run(output: Option<String>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let mut event_loop = EventLoop::<App>::try_new().map_err(|e| e.to_string())?;

    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| format!("the compositor has no layer-shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        pool: SlotPool::new(4096, &shm).map_err(|e| e.to_string())?,
        shm,
        layer: None,
        _region: None,
        draw: Draw::default(),
        pm: None,
        size: (0, 0),
        scale: 1,
        configured: false,
        dirty: false,
        frame_pending: false,
        exit: false,
    };

    // Outputs and their names arrive after the first round trips.
    let mut queue = queue;
    for _ in 0..2 {
        queue.roundtrip(&mut app).map_err(|e| e.to_string())?;
    }
    let outputs: Vec<_> = app
        .output_state
        .outputs()
        .map(|o| (app.output_state.info(&o), o))
        .collect();
    let chosen = output
        .as_deref()
        .and_then(|n| {
            outputs
                .iter()
                .find(|(i, _)| i.as_ref().and_then(|i| i.name.as_deref()) == Some(n))
        })
        .or_else(|| outputs.first());
    if let Some((info, _)) = chosen {
        app.scale = info.as_ref().map_or(1, |i| i.scale_factor).max(1);
    }

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("arrowhead-overlay"),
        chosen.map(|(_, o)| o),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // An empty input region: every click goes to the window below.
    let region = Region::new(&compositor).map_err(|e| e.to_string())?;
    layer.set_input_region(Some(region.wl_region()));
    layer.commit();
    app._region = Some(region);
    app.layer = Some(layer);

    let (tx, rx) = channel::channel::<Vec<Msg>>();
    event_loop
        .handle()
        .insert_source(rx, |ev, _, app: &mut App| match ev {
            Event::Msg(msgs) => {
                for m in msgs {
                    app.dirty |= app.draw.apply(m);
                }
                // The Wayland source dispatches queued events before this runs again; the
                // redraw itself happens in the loop callback below.
            }
            Event::Closed => app.exit = true,
        })
        .map_err(|e| e.to_string())?;
    WaylandSource::new(conn, queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    std::thread::spawn(move || crate::read_stdin(|m| tx.send(m).is_ok()));

    let signal = event_loop.get_signal();
    event_loop
        .run(None, &mut app, |app| {
            app.redraw(&qh);
            if app.exit {
                signal.stop();
            }
        })
        .map_err(|e| e.to_string())
}
