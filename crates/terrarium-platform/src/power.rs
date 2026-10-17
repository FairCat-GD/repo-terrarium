//! Windows' power notifications, read as the strip needs them: the machine woke, or its screens
//! came back on. Pure, so it is tested on every OS; the `windows` module subscribes.

use crate::appbar::PBT_APMRESUMEAUTOMATIC;

/// Whether a `WM_POWERBROADCAST` says the machine woke from sleep. `PBT_APMRESUMEAUTOMATIC`
/// comes on every resume; `PBT_APMRESUMESUSPEND` follows it only when a person woke it, so it
/// is not taken too.
pub fn resumed(wparam: usize) -> bool {
    wparam == PBT_APMRESUMEAUTOMATIC
}

/// The console display's state, as `GUID_CONSOLE_DISPLAY_STATE` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPower {
    Off,
    On,
    Dimmed,
}

impl DisplayPower {
    /// The setting's value: 0 off, 1 on, 2 dimmed; anything else is not read.
    pub fn from_setting(value: u32) -> Option<DisplayPower> {
        match value {
            0 => Some(DisplayPower::Off),
            1 => Some(DisplayPower::On),
            2 => Some(DisplayPower::Dimmed),
            _ => None,
        }
    }
}

/// The display's state after `now`, kept as the last state that was on or off (dimming is
/// on the way to off, or back), and whether the screens woke: off, then on. Windows sends the
/// current state as soon as the strip subscribes, which is never a wake.
pub fn next_display(last: Option<DisplayPower>, now: DisplayPower) -> (Option<DisplayPower>, bool) {
    match now {
        DisplayPower::Dimmed => (last, false),
        DisplayPower::On => (Some(now), last == Some(DisplayPower::Off)),
        DisplayPower::Off => (Some(now), false),
    }
}

/// A GUID's fields from its 16 bytes as Windows lays them in memory: the first three in the
/// machine's byte order, then the last eight bytes as they are.
pub fn guid_fields(bytes: [u8; 16]) -> (u32, u16, u16, [u8; 8]) {
    let [_, _, _, _, _, _, _, _, last @ ..] = bytes;
    (
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_ne_bytes([bytes[4], bytes[5]]),
        u16::from_ne_bytes([bytes[6], bytes[7]]),
        last,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_endian = "little")]
    fn the_display_state_guid_reads_from_its_bytes_in_memory() {
        // GUID_CONSOLE_DISPLAY_STATE, 6FE69556-704A-47A0-8F24-C28D936FDA47, as it lies in a
        // little-endian machine's memory.
        let bytes = [
            0x56, 0x95, 0xE6, 0x6F, 0x4A, 0x70, 0xA0, 0x47, 0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F,
            0xDA, 0x47,
        ];
        assert_eq!(
            guid_fields(bytes),
            (
                0x6FE6_9556,
                0x704A,
                0x47A0,
                [0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F, 0xDA, 0x47]
            )
        );
    }

    #[test]
    fn only_the_automatic_resume_is_a_wake() {
        assert!(resumed(0x12));
        assert!(!resumed(0x7), "it follows the automatic one");
        assert!(!resumed(0x4));
        assert!(!resumed(0x8013));
    }

    #[test]
    fn the_first_state_after_subscribing_is_not_a_wake() {
        assert_eq!(
            next_display(None, DisplayPower::On),
            (Some(DisplayPower::On), false)
        );
        assert_eq!(
            next_display(None, DisplayPower::Off),
            (Some(DisplayPower::Off), false)
        );
    }

    #[test]
    fn off_then_on_is_the_screens_waking() {
        let (last, woke) = next_display(Some(DisplayPower::On), DisplayPower::Off);
        assert!(!woke);
        assert_eq!(
            next_display(last, DisplayPower::On),
            (Some(DisplayPower::On), true)
        );
        assert_eq!(
            next_display(Some(DisplayPower::On), DisplayPower::On),
            (Some(DisplayPower::On), false)
        );
    }

    #[test]
    fn dimming_is_neither_and_remembers_what_came_before() {
        assert_eq!(
            next_display(Some(DisplayPower::Off), DisplayPower::Dimmed),
            (Some(DisplayPower::Off), false)
        );
        assert_eq!(
            next_display(Some(DisplayPower::On), DisplayPower::Dimmed),
            (Some(DisplayPower::On), false)
        );
        let (last, _) = next_display(Some(DisplayPower::Off), DisplayPower::Dimmed);
        assert!(next_display(last, DisplayPower::On).1);
        assert_eq!(DisplayPower::from_setting(1), Some(DisplayPower::On));
        assert_eq!(DisplayPower::from_setting(0), Some(DisplayPower::Off));
        assert_eq!(DisplayPower::from_setting(2), Some(DisplayPower::Dimmed));
        assert_eq!(DisplayPower::from_setting(3), None);
    }
}
