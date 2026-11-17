//! Platform v1: physical pixels, USB HID keyboard usages, no winit/native IDs.
//! Producers normalize OS scancodes and keep GL calls on the context's thread.
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakePolicy {
    Poll,
    Wait,
    WaitUntil(Instant),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerMode {
    Free,
    /// Hidden, captured, unbounded relative motion (not cursor-edge deltas).
    Relative,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    Close,
    /// Either zero axis means no drawable; keep ticking/saving, suspend drawing.
    Resize([u32; 2]),
    Focus(bool),
    /// USB HID usage page 0x07. Unmappable keys must not become fake winit IDs.
    Key {
        usage: u16,
        pressed: bool,
        repeat: bool,
    },
    Text(String),
    Pointer([f64; 2]),
    RelativeMotion([f64; 2]),
    Button {
        button: MouseButton,
        pressed: bool,
    },
    /// Wheel line units; adapters convert pixel wheels explicitly.
    Wheel([f32; 2]),
    /// Both this and queue overflow require clearing held input immediately.
    CaptureLost,
}

pub trait PlatformWindow {
    fn size(&self) -> [u32; 2];
    fn set_title(&mut self, title: &str) -> Result<(), String>;
    fn set_pointer_mode(&mut self, mode: PointerMode) -> Result<(), String>;
    /// Drain a bounded batch. Return lost event count separately; never pretend
    /// that an overflowed queue represents a complete input history.
    fn poll_events(&mut self, output: &mut Vec<WindowEvent>) -> Result<usize, String>;
}
