//! Physical keyboard producers: USB HID page 07, not layout-dependent letters.
//! Unknown native codes remain unmapped; no evdev keycode-minus-eight guess.
#[cfg(any(windows, test))]
pub(crate) fn win32(scan: u8, extended: bool, virtual_key: u32) -> Option<u16> {
    if virtual_key == 0x13 {
        return Some(0x48);
    } // Pause has an E1 sequence.
    if extended {
        return Some(match scan {
            0x1c => 0x58,
            0x1d => 0xe4,
            0x35 => 0x54,
            0x37 => 0x46,
            0x38 => 0xe6,
            0x47 => 0x4a,
            0x48 => 0x52,
            0x49 => 0x4b,
            0x4b => 0x50,
            0x4d => 0x4f,
            0x4f => 0x4d,
            0x50 => 0x51,
            0x51 => 0x4e,
            0x52 => 0x49,
            0x53 => 0x4c,
            0x5b => 0xe3,
            0x5c => 0xe7,
            0x5d => 0x65,
            _ => return None,
        });
    }
    Some(match scan {
        0x01 => 0x29,
        0x02..=0x0b => 0x1e + u16::from(scan - 2),
        0x0c => 0x2d,
        0x0d => 0x2e,
        0x0e => 0x2a,
        0x0f => 0x2b,
        0x10..=0x19 => {
            [0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13][usize::from(scan - 0x10)]
        }
        0x1a => 0x2f,
        0x1b => 0x30,
        0x1c => 0x28,
        0x1d => 0xe0,
        0x1e..=0x26 => {
            [0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f][usize::from(scan - 0x1e)]
        }
        0x27 => 0x33,
        0x28 => 0x34,
        0x29 => 0x35,
        0x2a => 0xe1,
        0x2b => 0x31,
        0x2c..=0x32 => [0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10][usize::from(scan - 0x2c)],
        0x33 => 0x36,
        0x34 => 0x37,
        0x35 => 0x38,
        0x36 => 0xe5,
        0x37 => 0x55,
        0x38 => 0xe2,
        0x39 => 0x2c,
        0x3a => 0x39,
        0x3b..=0x44 => 0x3a + u16::from(scan - 0x3b),
        0x45 => 0x53,
        0x46 => 0x47,
        0x47 => 0x5f,
        0x48 => 0x60,
        0x49 => 0x61,
        0x4a => 0x56,
        0x4b => 0x5c,
        0x4c => 0x5d,
        0x4d => 0x5e,
        0x4e => 0x57,
        0x4f => 0x59,
        0x50 => 0x5a,
        0x51 => 0x5b,
        0x52 => 0x62,
        0x53 => 0x63,
        0x56 => 0x64,
        0x57 => 0x44,
        0x58 => 0x45,
        _ => return None,
    })
}
#[cfg(any(target_os = "linux", test))]
pub(crate) fn xkb_name(name: [u8; 4]) -> Option<u16> {
    let name = name.split(|&b| b == 0).next()?;
    let row = match name.get(..2)? {
        b"AE" => Some(
            &[
                0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e,
            ][..],
        ),
        b"AD" => Some(
            &[
                0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30,
            ][..],
        ),
        b"AC" => Some(
            &[
                0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34,
            ][..],
        ),
        b"AB" => Some(&[0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38][..]),
        _ => None,
    };
    if let Some(row) = row {
        let n = std::str::from_utf8(name.get(2..)?)
            .ok()?
            .parse::<usize>()
            .ok()?;
        return row.get(n.checked_sub(1)?).copied();
    }
    if name.starts_with(b"FK") {
        let n = std::str::from_utf8(name.get(2..)?)
            .ok()?
            .parse::<u16>()
            .ok()?;
        return (1..=12).contains(&n).then_some(0x39 + n);
    }
    Some(match name {
        b"ESC" => 0x29,
        b"BKSP" => 0x2a,
        b"TAB" => 0x2b,
        b"RTRN" => 0x28,
        b"SPCE" => 0x2c,
        b"TLDE" => 0x35,
        b"BKSL" => 0x31,
        b"LSGT" => 0x64,
        b"CAPS" => 0x39,
        b"LCTL" => 0xe0,
        b"LFSH" => 0xe1,
        b"LALT" => 0xe2,
        b"LWIN" => 0xe3,
        b"RCTL" => 0xe4,
        b"RTSH" => 0xe5,
        b"RALT" => 0xe6,
        b"RWIN" => 0xe7,
        b"MENU" => 0x65,
        b"PRSC" => 0x46,
        b"SCLK" => 0x47,
        b"PAUS" => 0x48,
        b"INS" => 0x49,
        b"HOME" => 0x4a,
        b"PGUP" => 0x4b,
        b"DELE" => 0x4c,
        b"END" => 0x4d,
        b"PGDN" => 0x4e,
        b"RGHT" => 0x4f,
        b"LEFT" => 0x50,
        b"DOWN" => 0x51,
        b"UP" => 0x52,
        b"NMLK" => 0x53,
        b"KPDV" => 0x54,
        b"KPMU" => 0x55,
        b"KPSU" => 0x56,
        b"KPAD" => 0x57,
        b"KPEN" => 0x58,
        b"KP1" => 0x59,
        b"KP2" => 0x5a,
        b"KP3" => 0x5b,
        b"KP4" => 0x5c,
        b"KP5" => 0x5d,
        b"KP6" => 0x5e,
        b"KP7" => 0x5f,
        b"KP8" => 0x60,
        b"KP9" => 0x61,
        b"KP0" => 0x62,
        b"KPDL" => 0x63,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scancodes_not_virtual_letters_and_extended_keys_never_alias() {
        assert_eq!(win32(0x11, false, 0x5a), Some(0x1a)); // physical W, layout Z
        assert_eq!(win32(0x1c, false, 13), Some(0x28));
        assert_eq!(win32(0x1c, true, 13), Some(0x58));
        assert_eq!(win32(0x1d, false, 17), Some(0xe0));
        assert_eq!(win32(0x1d, true, 17), Some(0xe4));
        assert_eq!(win32(0x48, false, 0), Some(0x60));
        assert_eq!(win32(0x48, true, 0), Some(0x52));
        assert_eq!(win32(0, false, 0x41), None);
        assert_eq!(win32(0x45, false, 0x13), Some(0x48));
    }
    #[test]
    fn server_key_names_not_x11_keycode_or_keysym_define_physical_usage() {
        for (name, usage) in [
            (*b"AD02", 0x1a),
            (*b"AC01", 4),
            (*b"AB01", 0x1d),
            (*b"LCTL", 0xe0),
            (*b"RCTL", 0xe4),
            (*b"FK12", 0x45),
            (*b"KPEN", 0x58),
        ] {
            assert_eq!(xkb_name(name), Some(usage));
        }
        for name in [*b"AD00", *b"AE99", *b"FK00", *b"XXXX", [0; 4]] {
            assert_eq!(xkb_name(name), None);
        }
    }
}
