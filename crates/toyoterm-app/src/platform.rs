use super::AppEvent;
use std::cell::{Cell, RefCell};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}
impl<T> PhysicalSize<T> {
    pub fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}
impl<T> PhysicalPosition<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}
#[derive(Clone)]
pub(super) struct EventSender(pub async_channel::Sender<AppEvent>);
impl EventSender {
    pub fn send_event(&self, event: AppEvent) -> Result<(), async_channel::TrySendError<AppEvent>> {
        self.0.try_send(event)
    }
}
#[derive(Default)]
pub(super) struct AppControl {
    pub exiting: Cell<bool>,
    pub deadline: Cell<Option<Instant>>,
}
impl AppControl {
    pub fn exit(&self) {
        self.exiting.set(true);
    }
    pub fn set_control_flow(&self, flow: ControlFlow) {
        self.deadline.set(match flow {
            ControlFlow::Wait => None,
            ControlFlow::WaitUntil(t) => Some(t),
        });
    }
}
pub(super) enum ControlFlow {
    Wait,
    WaitUntil(Instant),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ElementState {
    Pressed,
    Released,
}
pub(super) enum MouseButton {
    Left,
}
pub(super) enum MouseScrollDelta {
    LineDelta(f32),
    PixelDelta(PhysicalPosition<f64>),
}
pub(super) enum Ime {
    Preedit(String, Option<(usize, usize)>),
    Commit(String),
    Disabled,
}
pub(super) enum WindowEvent {
    Resized(PhysicalSize<u32>),
    Focused(bool),
    CursorMoved {
        position: PhysicalPosition<f64>,
    },
    MouseInput {
        state: ElementState,
        button: MouseButton,
    },
    MouseWheel {
        delta: MouseScrollDelta,
    },
    KeyboardInput {
        event: KeyEvent,
    },
    Ime(Ime),
    RedrawRequested,
}
#[derive(Clone, Debug)]
pub(super) struct KeyEvent {
    pub logical_key: Key,
    pub text: Option<String>,
    pub state: ElementState,
    pub repeat: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Named(NamedKey),
    Character(String),
    Unidentified,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ModifiersState(u8);
impl ModifiersState {
    pub const SHIFT: Self = Self(1);
    pub const CONTROL: Self = Self(2);
    pub const ALT: Self = Self(4);
    pub const SUPER: Self = Self(8);
    #[cfg(test)]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn empty() -> Self {
        Self(0)
    }
    pub fn shift_key(self) -> bool {
        self.0 & 1 != 0
    }
    pub fn control_key(self) -> bool {
        self.0 & 2 != 0
    }
    pub fn alt_key(self) -> bool {
        self.0 & 4 != 0
    }
    pub fn super_key(self) -> bool {
        self.0 & 8 != 0
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for ModifiersState {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl From<gpui::Modifiers> for ModifiersState {
    fn from(m: gpui::Modifiers) -> Self {
        Self(
            (if m.shift { Self::SHIFT.0 } else { 0 })
                | (u8::from(m.control) << 1)
                | (u8::from(m.alt) << 2)
                | (if m.platform { Self::SUPER.0 } else { 0 }),
        )
    }
}
#[derive(Clone, Copy)]
pub(super) enum Fullscreen {
    Borderless(()),
}
pub(super) enum WindowCommand {
    Maximize(bool),
    Minimize,
    Fullscreen(bool),
}
/// Main-thread window snapshot and queued native operations, applied by the GPUI view.
pub(super) struct Window {
    pub size: Cell<PhysicalSize<u32>>,
    pub scale: Cell<f64>,
    pub redraw: Cell<bool>,
    pub title: RefCell<String>,
    pub commands: RefCell<Vec<WindowCommand>>,
    pub maximized: Cell<bool>,
    pub fullscreen: Cell<bool>,
    pub font_changed: Cell<bool>,
    pub ime_bounds: Cell<(PhysicalPosition<f64>, PhysicalSize<u32>)>,
}
impl Window {
    pub fn new(size: PhysicalSize<u32>, scale: f64) -> Self {
        Self {
            size: Cell::new(size),
            scale: Cell::new(scale),
            redraw: Cell::new(true),
            title: RefCell::new(String::new()),
            commands: RefCell::new(vec![]),
            maximized: Cell::new(false),
            fullscreen: Cell::new(false),
            font_changed: Cell::new(false),
            ime_bounds: Cell::new((PhysicalPosition::default(), PhysicalSize::default())),
        }
    }
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.size.get()
    }
    pub fn scale_factor(&self) -> f64 {
        self.scale.get()
    }
    pub fn request_redraw(&self) {
        self.redraw.set(true);
    }
    pub fn set_title(&self, title: &str) {
        if *self.title.borrow() != title {
            self.title.replace(title.into());
            self.request_redraw();
        }
    }
    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<u32>) {
        self.ime_bounds.set((position, size));
    }
    pub fn set_maximized(&self, value: bool) {
        self.commands
            .borrow_mut()
            .push(WindowCommand::Maximize(value));
    }
    pub fn is_maximized(&self) -> bool {
        self.maximized.get()
    }
    pub fn set_minimized(&self, value: bool) {
        if value {
            self.commands.borrow_mut().push(WindowCommand::Minimize);
        }
    }
    pub fn fullscreen(&self) -> Option<Fullscreen> {
        self.fullscreen.get().then_some(Fullscreen::Borderless(()))
    }
    pub fn set_fullscreen(&self, value: Option<Fullscreen>) {
        self.commands
            .borrow_mut()
            .push(WindowCommand::Fullscreen(value.is_some()));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NamedKey {
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Backspace,
    Delete,
    End,
    Enter,
    Escape,
    F1,
    F10,
    F11,
    F12,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    Home,
    Insert,
    PageDown,
    PageUp,
    Space,
    Tab,
}
