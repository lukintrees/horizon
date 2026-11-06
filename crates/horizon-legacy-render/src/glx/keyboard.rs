//! XKB C ABI declarations checked against MIT x11-rs at
//! ff57a942e5a4d35d8660362a8ca2ac0c02fa4852 (see THIRD_PARTY_NOTICES.md).
//! No keycode numbering assumptions: ask this live server for physical names.
use super::{Display, Library};
use std::ffi::{c_int, c_uint, c_ulong, c_void};
#[repr(C)]
struct Description {
    display: Display,
    flags: u16,
    device: u16,
    min: u8,
    max: u8,
    controls: *mut c_void,
    server: *mut c_void,
    map: *mut c_void,
    indicators: *mut c_void,
    names: *mut Names,
    compatibility: *mut c_void,
    geometry: *mut c_void,
}
#[repr(C)]
struct Names {
    keycodes: c_ulong,
    geometry: c_ulong,
    symbols: c_ulong,
    types: c_ulong,
    compatibility: c_ulong,
    virtual_modifiers: [c_ulong; 16],
    indicators: [c_ulong; 32],
    groups: [c_ulong; 4],
    keys: *const [u8; 4],
    aliases: *mut c_void,
    radio_groups: *mut c_ulong,
    physical_symbols: c_ulong,
    num_keys: u8,
    num_aliases: u8,
    num_radio_groups: u16,
}
pub(super) fn read(library: &Library, display: Display) -> Result<[u16; 256], String> {
    type GetMap = unsafe extern "C" fn(Display, c_uint, c_uint) -> *mut Description;
    type GetNames = unsafe extern "C" fn(Display, c_uint, *mut Description) -> c_int;
    type Free = unsafe extern "C" fn(*mut Description, c_uint, c_int);
    // SAFETY: fixed public Xlib symbols and their C signatures. The library
    // and display remain owned by Context throughout this synchronous copy.
    unsafe {
        let get_map: GetMap =
            std::mem::transmute::<*const c_void, GetMap>(library.symbol(c"XkbGetMap")?);
        let get_names: GetNames =
            std::mem::transmute::<*const c_void, GetNames>(library.symbol(c"XkbGetNames")?);
        let free: Free =
            std::mem::transmute::<*const c_void, Free>(library.symbol(c"XkbFreeKeyboard")?);
        let keyboard = get_map(display, 7, 0x100); // all client map data, core keyboard
        if keyboard.is_null() {
            return Err("native input requires XKB physical key names".into());
        }
        let result = (|| {
            if get_names(display, 1 << 9, keyboard) != 0 {
                return Err("XkbGetNames failed".into());
            }
            let desc = &*keyboard;
            let names = desc.names.as_ref().ok_or("XKB key names missing")?;
            if names.keys.is_null() || desc.min > desc.max {
                return Err("invalid XKB key names".into());
            }
            let mut usages = [0; 256];
            for code in desc.min..=desc.max {
                usages[usize::from(code)] =
                    crate::keyboard::xkb_name(*names.keys.add(usize::from(code))).unwrap_or(0);
            }
            if !usages.contains(&0x1a) || !usages.contains(&0x28) {
                return Err("XKB lacks physical movement/enter keys".into());
            }
            Ok(usages)
        })();
        free(keyboard, 0, 1);
        result
    }
}
