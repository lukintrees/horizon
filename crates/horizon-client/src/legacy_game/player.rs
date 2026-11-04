//! Player motion, collision, block selection and the small creative hotbar.

use serde::{Deserialize, Serialize};

use super::world::{Block, WORLD_HEIGHT, World, height_at};

pub const EYE_HEIGHT: f32 = 1.62;
pub const PLAYER_WIDTH: f32 = 0.30;
pub const PLAYER_HEIGHT: f32 = 1.80;
pub const REACH: f32 = 5.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Player {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: u8,
    pub hunger: u8,
    pub selected: u8,
    #[serde(default = "default_hotbar")]
    pub slots: [u8; 9],
    #[serde(default = "default_counts")]
    pub counts: [u8; 9],
    pub on_ground: bool,
}

fn default_hotbar() -> [u8; 9] {
    [
        Block::Grass.as_u8(),
        Block::Dirt.as_u8(),
        Block::Stone.as_u8(),
        Block::Sand.as_u8(),
        Block::Wood.as_u8(),
        Block::Leaves.as_u8(),
        Block::Planks.as_u8(),
        Block::Cobble.as_u8(),
        Block::Water.as_u8(),
    ]
}

fn default_counts() -> [u8; 9] {
    [64; 9]
}

impl Player {
    pub fn spawn(world: &mut World) -> Self {
        let mut y = (height_at(world.seed, 8, 8) + 3) as f32;
        while y < (WORLD_HEIGHT - 2) as f32 && world.get(8, y.floor() as i32, 8).solid() {
            y += 1.0;
        }
        Self {
            position: [8.5, y, 8.5],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: -0.08,
            health: 20,
            hunger: 20,
            selected: 0,
            slots: default_hotbar(),
            counts: default_counts(),
            on_ground: false,
        }
    }

    pub fn eye(&self) -> [f32; 3] {
        [
            self.position[0],
            self.position[1] + EYE_HEIGHT,
            self.position[2],
        ]
    }

    pub fn direction(&self) -> [f32; 3] {
        let cos_pitch = self.pitch.cos();
        [
            self.yaw.sin() * cos_pitch,
            self.pitch.sin(),
            -self.yaw.cos() * cos_pitch,
        ]
    }

    fn forward(&self) -> [f32; 2] {
        [self.yaw.sin(), -self.yaw.cos()]
    }

    fn right(&self) -> [f32; 2] {
        [self.yaw.cos(), self.yaw.sin()]
    }

    pub fn look(&mut self, dx: f32, dy: f32, sensitivity: f32) {
        self.yaw += dx * sensitivity;
        self.pitch = (self.pitch - dy * sensitivity).clamp(-1.55, 1.55);
    }

    pub fn selected_block(&self) -> Block {
        let slot = (self.selected as usize).min(8);
        Block::from_u8(self.slots[slot])
    }

    pub fn cycle_selection(&mut self, delta: i32) {
        let current = self.selected as i32;
        self.selected = (current + delta).rem_euclid(9) as u8;
    }

    pub fn set_selection(&mut self, slot: u8) {
        if slot < 9 {
            self.selected = slot;
        }
    }

    fn collides(&self, world: &mut World, position: [f32; 3]) -> bool {
        let min_x = (position[0] - PLAYER_WIDTH).floor() as i32;
        let max_x = (position[0] + PLAYER_WIDTH).floor() as i32;
        let min_y = position[1].floor() as i32;
        let max_y = (position[1] + PLAYER_HEIGHT).floor() as i32;
        let min_z = (position[2] - PLAYER_WIDTH).floor() as i32;
        let max_z = (position[2] + PLAYER_WIDTH).floor() as i32;
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    if world.get(x, y, z).solid() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn move_axis(&mut self, world: &mut World, axis: usize, amount: f32) {
        if amount == 0.0 {
            return;
        }
        let steps = (amount.abs() / 0.12).ceil().max(1.0) as usize;
        let step = amount / steps as f32;
        for _ in 0..steps {
            let mut next = self.position;
            next[axis] += step;
            if !self.collides(world, next) {
                self.position = next;
            } else {
                self.velocity[axis] = 0.0;
                if axis == 1 && step < 0.0 {
                    self.on_ground = true;
                }
                break;
            }
        }
    }

    pub fn update(&mut self, world: &mut World, input: &Input, dt: f32) {
        let forward = self.forward();
        let right = self.right();
        let mut move_x = 0.0;
        let mut move_z = 0.0;
        if input.forward {
            move_x += forward[0];
            move_z += forward[1];
        }
        if input.back {
            move_x -= forward[0];
            move_z -= forward[1];
        }
        if input.right {
            move_x += right[0];
            move_z += right[1];
        }
        if input.left {
            move_x -= right[0];
            move_z -= right[1];
        }
        let length = (move_x * move_x + move_z * move_z).sqrt();
        if length > 0.0 {
            move_x /= length;
            move_z /= length;
        }
        let speed = if input.sprint {
            7.2
        } else if input.sneak {
            1.8
        } else {
            4.3
        };
        self.velocity[0] = move_x * speed;
        self.velocity[2] = move_z * speed;
        if input.jump && self.on_ground {
            self.velocity[1] = 7.4;
            self.on_ground = false;
        }
        self.velocity[1] = (self.velocity[1] - 22.0 * dt).max(-42.0);
        self.on_ground = false;
        self.move_axis(world, 1, self.velocity[1] * dt);
        self.move_axis(world, 0, self.velocity[0] * dt);
        self.move_axis(world, 2, self.velocity[2] * dt);
        if self.position[1] < -8.0 {
            *self = Self::spawn(world);
        }
    }

    /// Amanatides-Woo DDA. Returns the hit block and the face normal.
    pub fn raycast(&self, world: &mut World, reach: f32) -> Option<([i32; 3], [i32; 3])> {
        let origin = self.eye();
        let direction = self.direction();
        let mut x = origin[0].floor() as i32;
        let mut y = origin[1].floor() as i32;
        let mut z = origin[2].floor() as i32;
        let step = [
            direction[0].signum() as i32,
            direction[1].signum() as i32,
            direction[2].signum() as i32,
        ];
        let delta = [
            if direction[0].abs() > 1e-6 {
                (1.0 / direction[0].abs()).min(1e6)
            } else {
                1e6
            },
            if direction[1].abs() > 1e-6 {
                (1.0 / direction[1].abs()).min(1e6)
            } else {
                1e6
            },
            if direction[2].abs() > 1e-6 {
                (1.0 / direction[2].abs()).min(1e6)
            } else {
                1e6
            },
        ];
        let boundary = [
            if step[0] > 0 {
                (x + 1) as f32 - origin[0]
            } else {
                origin[0] - x as f32
            },
            if step[1] > 0 {
                (y + 1) as f32 - origin[1]
            } else {
                origin[1] - y as f32
            },
            if step[2] > 0 {
                (z + 1) as f32 - origin[2]
            } else {
                origin[2] - z as f32
            },
        ];
        let mut max = [
            boundary[0] * delta[0],
            boundary[1] * delta[1],
            boundary[2] * delta[2],
        ];
        let mut normal: [i32; 3];
        for _ in 0..256 {
            let axis = if max[0] <= max[1] && max[0] <= max[2] {
                0
            } else if max[1] <= max[2] {
                1
            } else {
                2
            };
            if max[axis] > reach {
                return None;
            }
            match axis {
                0 => x += step[0],
                1 => y += step[1],
                _ => z += step[2],
            }
            max[axis] += delta[axis];
            normal = [0, 0, 0];
            normal[axis] = -step[axis];
            let block = world.get(x, y, z);
            if block != Block::Air && block != Block::Water {
                return Some(([x, y, z], normal));
            }
        }
        None
    }

    pub fn break_block(&mut self, world: &mut World) -> bool {
        if let Some((pos, _)) = self.raycast(world, REACH) {
            world.set(pos[0], pos[1], pos[2], Block::Air);
            return true;
        }
        false
    }

    pub fn place_block(&mut self, world: &mut World) -> bool {
        let slot = self.selected as usize;
        if self.counts[slot] == 0 {
            return false;
        }
        if let Some((pos, normal)) = self.raycast(world, REACH) {
            let target = [pos[0] + normal[0], pos[1] + normal[1], pos[2] + normal[2]];
            if !world.get(target[0], target[1], target[2]).solid() && !self.body_intersects(target)
            {
                world.set(target[0], target[1], target[2], self.selected_block());
                self.counts[slot] = self.counts[slot].saturating_sub(1);
                return true;
            }
        }
        false
    }

    fn body_intersects(&self, block: [i32; 3]) -> bool {
        let min_x = (self.position[0] - PLAYER_WIDTH).floor() as i32;
        let max_x = (self.position[0] + PLAYER_WIDTH).floor() as i32;
        let min_y = self.position[1].floor() as i32;
        let max_y = (self.position[1] + PLAYER_HEIGHT).floor() as i32;
        let min_z = (self.position[2] - PLAYER_WIDTH).floor() as i32;
        let max_z = (self.position[2] + PLAYER_WIDTH).floor() as i32;
        block[0] >= min_x
            && block[0] <= max_x
            && block[1] >= min_y
            && block[1] <= max_y
            && block[2] >= min_z
            && block[2] <= max_z
    }
}
