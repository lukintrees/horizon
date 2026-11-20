//! Game-owned input data, independent of a window/device identity. Both the winit window and the
//! native desktop-GL window produce these, and both enter the same handlers: never manufacture
//! winit's private platform state to fake one.
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceId, ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta, TouchPhase},
    keyboard::{Key, PhysicalKey, SmolStr},
};

/// A key press, as the game reads one. `winit::event::KeyEvent` cannot be built outside winit
/// (`platform_specific` is private) and the native window is not a winit window, so the game
/// keeps the fields it uses in its own type; `From` converts the winit one.
#[derive(Clone, Debug)]
pub(super) struct KeyEvent {
    pub physical_key: PhysicalKey,
    pub logical_key: Key,
    pub text: Option<SmolStr>,
    pub state: ElementState,
    pub repeat: bool,
}

impl KeyEvent {
    /// `KeyEventExtModifierSupplement::key_without_modifiers`, as far as the fields kept here
    /// reach: the logical key the producer read. A native producer reports the keysym's own
    /// character, which is the same reading for every key but one held with a modifier.
    pub(super) fn key_without_modifiers(&self) -> Key {
        self.logical_key.clone()
    }
}

impl From<winit::event::KeyEvent> for KeyEvent {
    fn from(event: winit::event::KeyEvent) -> Self {
        Self {
            physical_key: event.physical_key,
            logical_key: event.logical_key,
            text: event.text,
            state: event.state,
            repeat: event.repeat,
        }
    }
}

/// The window events the game's handlers act on, whoever produced them. The variants and their
/// fields are winit's, so a handler written for one producer reads the other's events unchanged.
/// A producer's identity and the wheel's phase are carried but never matched on: a handler that
/// wants them reads the same names on both producers' events.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "fields keep winit's variant shapes so one handler serves both producers")]
pub(super) enum Event {
    CloseRequested,
    Resized(PhysicalSize<u32>),
    RedrawRequested,
    Focused(bool),
    DroppedFile(std::path::PathBuf),
    ModifiersChanged(Modifiers),
    KeyboardInput {
        device_id: DeviceId,
        event: KeyEvent,
        is_synthetic: bool,
    },
    MouseInput {
        device_id: DeviceId,
        state: ElementState,
        button: MouseButton,
    },
    MouseWheel {
        device_id: DeviceId,
        delta: MouseScrollDelta,
        phase: TouchPhase,
    },
    CursorMoved {
        device_id: DeviceId,
        position: PhysicalPosition<f64>,
    },
    Ime(Ime),
    Other,
}

impl From<winit::event::WindowEvent> for Event {
    fn from(event: winit::event::WindowEvent) -> Self {
        use winit::event::WindowEvent as W;
        match event {
            W::CloseRequested => Self::CloseRequested,
            W::Resized(size) => Self::Resized(size),
            W::RedrawRequested => Self::RedrawRequested,
            W::Focused(focused) => Self::Focused(focused),
            W::DroppedFile(file) => Self::DroppedFile(file),
            W::ModifiersChanged(modifiers) => Self::ModifiersChanged(modifiers),
            W::KeyboardInput {
                device_id,
                event,
                is_synthetic,
            } => Self::KeyboardInput {
                device_id,
                event: event.into(),
                is_synthetic,
            },
            W::MouseInput {
                device_id,
                state,
                button,
            } => Self::MouseInput {
                device_id,
                state,
                button,
            },
            W::MouseWheel {
                device_id,
                delta,
                phase,
            } => Self::MouseWheel {
                device_id,
                delta,
                phase,
            },
            W::CursorMoved {
                device_id,
                position,
            } => Self::CursorMoved {
                device_id,
                position,
            },
            W::Ime(ime) => Self::Ime(ime),
            _ => Self::Other,
        }
    }
}
