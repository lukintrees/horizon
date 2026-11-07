//! Game-owned input data, independent of a window/device identity. Both native
//! and winit producers enter the same handlers; never manufacture winit IDs or
//! the private platform state inside winit::event::KeyEvent.
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, Modifiers, MouseButton, MouseScrollDelta},
    keyboard::{Key, PhysicalKey},
};

#[derive(Clone, Debug)]
pub(super) struct KeyEvent {
    pub physical_key: PhysicalKey,
    pub logical_key: Key,
    pub text: Option<String>,
    pub state: ElementState,
    pub repeat: bool,
}
impl From<winit::event::KeyEvent> for KeyEvent {
    fn from(event: winit::event::KeyEvent) -> Self {
        Self {
            physical_key: event.physical_key,
            logical_key: event.logical_key,
            text: event.text.map(|s| s.to_string()),
            state: event.state,
            repeat: event.repeat,
        }
    }
}
#[derive(Debug)]
pub(super) enum Event {
    CloseRequested,
    Resized(PhysicalSize<u32>),
    RedrawRequested,
    Focused(bool),
    DroppedFile(std::path::PathBuf),
    ModifiersChanged(Modifiers),
    KeyboardInput {
        event: KeyEvent,
        is_synthetic: bool,
    },
    MouseInput {
        state: ElementState,
        button: MouseButton,
    },
    MouseWheel {
        delta: MouseScrollDelta,
    },
    CursorMoved {
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
                event,
                is_synthetic,
                ..
            } => Self::KeyboardInput {
                event: event.into(),
                is_synthetic,
            },
            W::MouseInput { state, button, .. } => Self::MouseInput { state, button },
            W::MouseWheel { delta, .. } => Self::MouseWheel { delta },
            W::CursorMoved { position, .. } => Self::CursorMoved { position },
            W::Ime(ime) => Self::Ime(ime),
            _ => Self::Other,
        }
    }
}
