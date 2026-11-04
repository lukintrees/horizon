//! Screen layout for the GL2 single-player client. All visible labels come
//! from bundled translation keys; no English UI text is hardcoded here.

use horizon_client_core::lang;

use super::world::Block;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    Playing,
    Pause,
    Inventory,
    Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    NewWorld,
    LoadWorld,
    Resume,
    Inventory,
    Options,
    SaveQuit,
    Quit,
    Back,
    IncreaseDistance,
    DecreaseDistance,
    IncreaseFov,
    DecreaseFov,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub color: [u8; 4],
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub scale: f32,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub rect: Rect,
    pub key: &'static str,
    pub action: UiAction,
}

impl Button {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiFrame {
    pub rects: Vec<Rect>,
    pub texts: Vec<TextLine>,
    pub buttons: Vec<Button>,
}

impl UiFrame {
    pub fn action_at(&self, x: f32, y: f32) -> Option<UiAction> {
        self.buttons
            .iter()
            .find(|button| button.contains(x, y))
            .map(|button| button.action)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiSnapshot {
    pub screen: Screen,
    pub width: f32,
    pub height: f32,
    pub fps: f32,
    pub position: [f32; 3],
    pub chunks: usize,
    pub draws: usize,
    pub day_time: u32,
    pub seed: u64,
    pub health: u8,
    pub hunger: u8,
    pub selected: u8,
    pub slots: [u8; 9],
    pub counts: [u8; 9],
    pub render_distance: i32,
    pub fov: f32,
    pub sensitivity: f32,
}

const PANEL: [u8; 4] = [16, 20, 28, 220];
const BUTTON: [u8; 4] = [48, 64, 92, 240];
const BUTTON_HOVER: [u8; 4] = [72, 96, 136, 250];
const TEXT: [u8; 4] = [235, 240, 248, 255];
const ACCENT: [u8; 4] = [126, 180, 255, 255];
const HOTBAR: [u8; 4] = [32, 36, 44, 220];
const SELECTED: [u8; 4] = [220, 220, 220, 255];

fn text(x: f32, y: f32, key: &str, scale: f32, color: [u8; 4]) -> TextLine {
    TextLine {
        x,
        y,
        text: lang::get(key),
        scale,
        color,
    }
}

fn translated(x: f32, y: f32, key: &str, args: &[&dyn std::fmt::Display], scale: f32) -> TextLine {
    TextLine {
        x,
        y,
        text: lang::tr(key, args),
        scale,
        color: TEXT,
    }
}

fn button(x: f32, y: f32, w: f32, h: f32, key: &'static str, action: UiAction) -> Button {
    Button {
        rect: Rect {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
            color: BUTTON,
        },
        key,
        action,
    }
}

impl UiFrame {
    pub fn build(state: &UiSnapshot) -> Self {
        let mut frame = Self {
            rects: Vec::new(),
            texts: Vec::new(),
            buttons: Vec::new(),
        };
        let w = state.width.max(1.0);
        let h = state.height.max(1.0);
        let center = w * 0.5;
        match state.screen {
            Screen::Title => {
                frame.rects.push(Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: w,
                    y1: h,
                    color: PANEL,
                });
                frame.texts.push(text(
                    center - 150.0,
                    80.0,
                    "horizon.legacyGl.game.title",
                    30.0,
                    ACCENT,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    180.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.newWorld",
                    UiAction::NewWorld,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    230.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.loadWorld",
                    UiAction::LoadWorld,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    280.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.options",
                    UiAction::Options,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    330.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.quit",
                    UiAction::Quit,
                ));
                frame.texts.push(text(
                    20.0,
                    h - 30.0,
                    "horizon.legacyGl.game.controls",
                    12.0,
                    TEXT,
                ));
            }
            Screen::Playing => {
                frame.hud(state);
            }
            Screen::Pause => {
                frame.hud(state);
                frame.rects.push(Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: w,
                    y1: h,
                    color: PANEL,
                });
                frame.texts.push(text(
                    center - 90.0,
                    90.0,
                    "horizon.legacyGl.game.paused",
                    26.0,
                    ACCENT,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    170.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.resume",
                    UiAction::Resume,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    220.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.inventory",
                    UiAction::Inventory,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    270.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.options",
                    UiAction::Options,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    320.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.saveQuit",
                    UiAction::SaveQuit,
                ));
                frame.buttons.push(button(
                    center - 110.0,
                    370.0,
                    220.0,
                    38.0,
                    "horizon.legacyGl.game.quit",
                    UiAction::Quit,
                ));
            }
            Screen::Inventory => {
                frame.hud(state);
                frame.rects.push(Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: w,
                    y1: h,
                    color: PANEL,
                });
                frame.texts.push(text(
                    center - 100.0,
                    70.0,
                    "horizon.legacyGl.game.inventory",
                    26.0,
                    ACCENT,
                ));
                for slot in 0..9 {
                    let x = center - 220.0 + slot as f32 * 50.0;
                    let block = Block::from_u8(state.slots[slot]);
                    frame.rects.push(Rect {
                        x0: x,
                        y0: 140.0,
                        x1: x + 44.0,
                        y1: 184.0,
                        color: HOTBAR,
                    });
                    frame
                        .texts
                        .push(translated(x + 4.0, 146.0, block.name_key(), &[], 11.0));
                    frame.texts.push(translated(
                        x + 4.0,
                        162.0,
                        "horizon.legacyGl.game.count",
                        &[&state.counts[slot]],
                        11.0,
                    ));
                }
                frame.buttons.push(button(
                    center - 80.0,
                    220.0,
                    160.0,
                    36.0,
                    "horizon.legacyGl.game.back",
                    UiAction::Back,
                ));
            }
            Screen::Options => {
                frame.rects.push(Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: w,
                    y1: h,
                    color: PANEL,
                });
                frame.texts.push(text(
                    center - 90.0,
                    70.0,
                    "horizon.legacyGl.game.options",
                    26.0,
                    ACCENT,
                ));
                frame.texts.push(translated(
                    center - 190.0,
                    140.0,
                    "horizon.legacyGl.game.renderDistance",
                    &[&state.render_distance],
                    16.0,
                ));
                frame.buttons.push(button(
                    center + 20.0,
                    132.0,
                    40.0,
                    30.0,
                    "horizon.legacyGl.game.minus",
                    UiAction::DecreaseDistance,
                ));
                frame.buttons.push(button(
                    center + 70.0,
                    132.0,
                    40.0,
                    30.0,
                    "horizon.legacyGl.game.plus",
                    UiAction::IncreaseDistance,
                ));
                frame.texts.push(translated(
                    center - 190.0,
                    190.0,
                    "horizon.legacyGl.game.fov",
                    &[&state.fov],
                    16.0,
                ));
                frame.buttons.push(button(
                    center + 20.0,
                    182.0,
                    40.0,
                    30.0,
                    "horizon.legacyGl.game.minus",
                    UiAction::DecreaseFov,
                ));
                frame.buttons.push(button(
                    center + 70.0,
                    182.0,
                    40.0,
                    30.0,
                    "horizon.legacyGl.game.plus",
                    UiAction::IncreaseFov,
                ));
                frame.texts.push(translated(
                    center - 190.0,
                    240.0,
                    "horizon.legacyGl.game.sensitivity",
                    &[&state.sensitivity],
                    16.0,
                ));
                frame.buttons.push(button(
                    center - 90.0,
                    300.0,
                    180.0,
                    36.0,
                    "horizon.legacyGl.game.back",
                    UiAction::Back,
                ));
            }
        }
        for line in &frame.buttons {
            frame.texts.push(text(
                line.rect.x0 + 10.0,
                line.rect.y0 + 10.0,
                line.key,
                14.0,
                TEXT,
            ));
        }
        frame
    }

    fn hud(&mut self, state: &UiSnapshot) {
        let w = state.width.max(1.0);
        let h = state.height.max(1.0);
        let center = w * 0.5;
        self.rects.push(Rect {
            x0: center - 6.0,
            y0: h * 0.5 - 1.0,
            x1: center + 6.0,
            y1: h * 0.5 + 1.0,
            color: SELECTED,
        });
        self.rects.push(Rect {
            x0: center - 1.0,
            y0: h * 0.5 - 6.0,
            x1: center + 1.0,
            y1: h * 0.5 + 6.0,
            color: SELECTED,
        });
        let hotbar_w = 9.0 * 42.0;
        let hotbar_x = center - hotbar_w * 0.5;
        self.rects.push(Rect {
            x0: hotbar_x - 4.0,
            y0: h - 54.0,
            x1: hotbar_x + hotbar_w + 4.0,
            y1: h - 6.0,
            color: HOTBAR,
        });
        for slot in 0_usize..9 {
            let slot_u8 = slot as u8;
            let x = hotbar_x + slot as f32 * 42.0;
            let color = if slot_u8 == state.selected {
                SELECTED
            } else {
                BUTTON_HOVER
            };
            self.rects.push(Rect {
                x0: x,
                y0: h - 50.0,
                x1: x + 38.0,
                y1: h - 10.0,
                color,
            });
            let block = Block::from_u8(state.slots[slot]);
            self.texts
                .push(translated(x + 3.0, h - 44.0, block.name_key(), &[], 9.0));
            self.texts.push(translated(
                x + 3.0,
                h - 28.0,
                "horizon.legacyGl.game.count",
                &[&state.counts[slot]],
                9.0,
            ));
        }
        let fps = state.fps.round() as i32;
        self.texts.push(translated(
            12.0,
            12.0,
            "horizon.legacyGl.game.fps",
            &[&fps],
            13.0,
        ));
        self.texts.push(translated(
            12.0,
            32.0,
            "horizon.legacyGl.game.position",
            &[&state.position[0], &state.position[1], &state.position[2]],
            12.0,
        ));
        self.texts.push(translated(
            12.0,
            50.0,
            "horizon.legacyGl.game.chunks",
            &[&state.chunks],
            12.0,
        ));
        self.texts.push(translated(
            12.0,
            68.0,
            "horizon.legacyGl.game.draws",
            &[&state.draws],
            12.0,
        ));
        self.texts.push(translated(
            12.0,
            86.0,
            "horizon.legacyGl.game.day",
            &[&state.day_time],
            12.0,
        ));
        self.texts.push(translated(
            12.0,
            104.0,
            "horizon.legacyGl.game.seed",
            &[&state.seed],
            12.0,
        ));
        self.texts.push(translated(
            12.0,
            h - 74.0,
            "horizon.legacyGl.game.health",
            &[&state.health],
            12.0,
        ));
        self.texts.push(translated(
            140.0,
            h - 74.0,
            "horizon.legacyGl.game.hunger",
            &[&state.hunger],
            12.0,
        ));
    }
}
