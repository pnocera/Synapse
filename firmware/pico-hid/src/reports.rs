pub const BOOT_MOUSE_REPORT_LEN: usize = 4;
pub const BOOT_KEYBOARD_REPORT_LEN: usize = 8;
pub const GAMEPAD_REPORT_LEN: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootMouseReport {
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

impl BootMouseReport {
    pub const fn neutral() -> Self {
        Self {
            buttons: 0,
            x: 0,
            y: 0,
            wheel: 0,
        }
    }

    pub const fn to_bytes(self) -> [u8; BOOT_MOUSE_REPORT_LEN] {
        [self.buttons, self.x as u8, self.y as u8, self.wheel as u8]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootKeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keycodes: [u8; 6],
}

impl BootKeyboardReport {
    pub const fn neutral() -> Self {
        Self {
            modifiers: 0,
            reserved: 0,
            keycodes: [0; 6],
        }
    }

    pub const fn to_bytes(self) -> [u8; BOOT_KEYBOARD_REPORT_LEN] {
        [
            self.modifiers,
            self.reserved,
            self.keycodes[0],
            self.keycodes[1],
            self.keycodes[2],
            self.keycodes[3],
            self.keycodes[4],
            self.keycodes[5],
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GamepadReport {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
    pub reserved: u16,
}

impl GamepadReport {
    pub const fn neutral() -> Self {
        Self {
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
            reserved: 0,
        }
    }

    pub fn to_bytes(self) -> [u8; GAMEPAD_REPORT_LEN] {
        let buttons = self.buttons.to_le_bytes();
        let thumb_lx = self.thumb_lx.to_le_bytes();
        let thumb_ly = self.thumb_ly.to_le_bytes();
        let thumb_rx = self.thumb_rx.to_le_bytes();
        let thumb_ry = self.thumb_ry.to_le_bytes();
        let reserved = self.reserved.to_le_bytes();

        [
            buttons[0],
            buttons[1],
            self.left_trigger,
            self.right_trigger,
            thumb_lx[0],
            thumb_lx[1],
            thumb_ly[0],
            thumb_ly[1],
            thumb_rx[0],
            thumb_rx[1],
            thumb_ry[0],
            thumb_ry[1],
            reserved[0],
            reserved[1],
        ]
    }

    pub fn from_bytes(bytes: [u8; GAMEPAD_REPORT_LEN]) -> Self {
        Self {
            buttons: u16::from_le_bytes([bytes[0], bytes[1]]),
            left_trigger: bytes[2],
            right_trigger: bytes[3],
            thumb_lx: i16::from_le_bytes([bytes[4], bytes[5]]),
            thumb_ly: i16::from_le_bytes([bytes[6], bytes[7]]),
            thumb_rx: i16::from_le_bytes([bytes[8], bytes[9]]),
            thumb_ry: i16::from_le_bytes([bytes[10], bytes[11]]),
            reserved: u16::from_le_bytes([bytes[12], bytes[13]]),
        }
    }
}
