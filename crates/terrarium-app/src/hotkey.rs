//! The global hotkeys (`hotkeys.*`): one raises the strip, one opens the
//! time-lapse. A binding is the person's to choose, within what the OS and other apps leave
//! free; it is stored in a form of its own, `ctrl+alt+T`, that no library's spelling can change.
//! Their presses are forwarded to the event loop as they happen; nothing polls for them.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::event_loop::EventLoopProxy;

use crate::strip::UserEvent;

/// Sends hotkey presses to the event loop. global-hotkey keeps the first handler it is given,
/// so this runs before any hotkey is registered.
pub fn forward_events(proxy: EventLoopProxy<UserEvent>) {
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let _ = proxy.send_event(UserEvent::HotKey(event));
    }));
}

/// Which hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Which {
    Raise,
    TimeLapse,
}

impl Which {
    pub const ALL: [Which; 2] = [Which::Raise, Which::TimeLapse];

    /// `Ctrl+Alt+T` raises the strip (Control-Option-T on a Mac) and `Ctrl+Alt+H` opens the
    /// time-lapse: Control keeps both inside macOS 15's rules, and neither letter is one a
    /// common keyboard layout types with `AltGr`.
    pub const fn default_binding(self) -> Binding {
        let mods = Mods {
            control: true,
            option: true,
            shift: false,
            command: false,
        };
        match self {
            Which::Raise => Binding {
                mods,
                key: Code::KeyT,
            },
            Which::TimeLapse => Binding {
                mods,
                key: Code::KeyH,
            },
        }
    }

    pub const fn other(self) -> Which {
        match self {
            Which::Raise => Which::TimeLapse,
            Which::TimeLapse => Which::Raise,
        }
    }
}

/// A binding's modifiers. Option is Alt on Windows, and Command the Windows key.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one switch per modifier, as the settings show them: any of the four may be on"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mods {
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub command: bool,
}

impl Mods {
    fn count(self) -> usize {
        [self.control, self.option, self.shift, self.command]
            .into_iter()
            .filter(|&on| on)
            .count()
    }

    fn modifiers(self) -> Modifiers {
        [
            (self.control, Modifiers::CONTROL),
            (self.option, Modifiers::ALT),
            (self.shift, Modifiers::SHIFT),
            (self.command, Modifiers::SUPER),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(Modifiers::empty(), |all, (_, one)| all | one)
    }
}

/// A key and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub mods: Mods,
    pub key: Code,
}

/// The keys a binding may end in, each with its name in the stored form: the letters, the
/// digits, F1 to F12 and the space bar. The words name them through `copy::key_word`.
pub const KEYS: [(Code, &str); 49] = [
    (Code::KeyA, "A"),
    (Code::KeyB, "B"),
    (Code::KeyC, "C"),
    (Code::KeyD, "D"),
    (Code::KeyE, "E"),
    (Code::KeyF, "F"),
    (Code::KeyG, "G"),
    (Code::KeyH, "H"),
    (Code::KeyI, "I"),
    (Code::KeyJ, "J"),
    (Code::KeyK, "K"),
    (Code::KeyL, "L"),
    (Code::KeyM, "M"),
    (Code::KeyN, "N"),
    (Code::KeyO, "O"),
    (Code::KeyP, "P"),
    (Code::KeyQ, "Q"),
    (Code::KeyR, "R"),
    (Code::KeyS, "S"),
    (Code::KeyT, "T"),
    (Code::KeyU, "U"),
    (Code::KeyV, "V"),
    (Code::KeyW, "W"),
    (Code::KeyX, "X"),
    (Code::KeyY, "Y"),
    (Code::KeyZ, "Z"),
    (Code::Digit0, "0"),
    (Code::Digit1, "1"),
    (Code::Digit2, "2"),
    (Code::Digit3, "3"),
    (Code::Digit4, "4"),
    (Code::Digit5, "5"),
    (Code::Digit6, "6"),
    (Code::Digit7, "7"),
    (Code::Digit8, "8"),
    (Code::Digit9, "9"),
    (Code::F1, "F1"),
    (Code::F2, "F2"),
    (Code::F3, "F3"),
    (Code::F4, "F4"),
    (Code::F5, "F5"),
    (Code::F6, "F6"),
    (Code::F7, "F7"),
    (Code::F8, "F8"),
    (Code::F9, "F9"),
    (Code::F10, "F10"),
    (Code::F11, "F11"),
    (Code::F12, "F12"),
    (Code::Space, "Space"),
];

/// The stored form's names of the modifiers, in its order.
const MOD_NAMES: [&str; 4] = ["ctrl", "alt", "shift", "super"];

/// The name `key` has in [`KEYS`].
pub fn key_name(key: Code) -> &'static str {
    KEYS.iter()
        .find(|(code, _)| *code == key)
        .map_or("?", |(_, name)| name)
}

/// Why a binding is refused before the OS is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A key alone, or with one modifier, is a key some app or text field uses.
    TooFewModifiers,
    /// Option and Shift alone: macOS 15 refuses them, and they type characters elsewhere.
    OptionShift,
    /// The other hotkey has it.
    Taken,
}

impl Binding {
    /// The stored form: the modifiers in a fixed order, then the key's name, `+` between.
    pub fn to_setting(self) -> String {
        let mods = self.mods;
        let mut parts: Vec<&str> = [mods.control, mods.option, mods.shift, mods.command]
            .into_iter()
            .zip(MOD_NAMES)
            .filter(|(on, _)| *on)
            .map(|(_, name)| name)
            .collect();
        parts.push(key_name(self.key));
        parts.join("+")
    }

    /// Reads the stored form: modifiers in any order, each once, then a key of [`KEYS`].
    pub fn parse(text: &str) -> Option<Binding> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = parts.pop()?;
        let key = KEYS
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(key))?
            .0;
        let mut mods = Mods::default();
        for part in parts {
            let flag = match part {
                "ctrl" => &mut mods.control,
                "alt" => &mut mods.option,
                "shift" => &mut mods.shift,
                "super" => &mut mods.command,
                _ => return None,
            };
            if std::mem::replace(flag, true) {
                return None;
            }
        }
        Some(Binding { mods, key })
    }

    /// Whether this binding can be offered to the OS beside `other`, the other hotkey's.
    pub fn check(self, other: Binding) -> Result<(), Refusal> {
        self.allowed()?;
        if self == other {
            return Err(Refusal::Taken);
        }
        Ok(())
    }

    /// Whether this binding keeps the rules whatever the other hotkey is.
    pub fn allowed(self) -> Result<(), Refusal> {
        let mods = self.mods;
        if mods.count() < 2 {
            return Err(Refusal::TooFewModifiers);
        }
        if !mods.control && !mods.command {
            return Err(Refusal::OptionShift);
        }
        Ok(())
    }

    fn hotkey(self) -> HotKey {
        HotKey::new(Some(self.mods.modifiers()), self.key)
    }
}

/// How the words name a binding's keys: as a Mac's keyboard does, or as Windows'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
}

impl Platform {
    pub const THIS: Platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        Platform::Windows
    };
}

/// The registered hotkeys, each bound on its own, so one the OS refuses never costs the other.
/// Dropping it unregisters them.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    bound: Bound,
}

impl Hotkeys {
    /// Must run on the main thread.
    pub fn new() -> Result<Hotkeys, global_hotkey::Error> {
        Ok(Hotkeys {
            manager: GlobalHotKeyManager::new()?,
            bound: Bound::default(),
        })
    }

    /// Registers `binding` for `which`. The new key is taken before the old one is let go, so
    /// a refusal leaves the hotkey as it was. Keys the other hotkey holds are refused: they
    /// stay its own.
    pub fn bind(&mut self, which: Which, binding: Binding) -> Result<(), global_hotkey::Error> {
        let hotkey = binding.hotkey();
        if self.bound.get(which) == Some(hotkey) {
            return Ok(());
        }
        if self.bound.get(which.other()) == Some(hotkey) {
            return Err(global_hotkey::Error::AlreadyRegistered(hotkey));
        }
        self.manager.register(hotkey)?;
        if let Some(old) = self.bound.slot(which).replace(hotkey) {
            let _ = self.manager.unregister(old);
        }
        Ok(())
    }

    /// The hotkey a press is for, if it is a press of one.
    pub fn which(&self, event: GlobalHotKeyEvent) -> Option<Which> {
        self.bound.which(event)
    }
}

/// Which keys each hotkey holds: how a press is told apart, without the OS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Bound {
    raise: Option<HotKey>,
    timelapse: Option<HotKey>,
}

impl Bound {
    fn get(self, which: Which) -> Option<HotKey> {
        match which {
            Which::Raise => self.raise,
            Which::TimeLapse => self.timelapse,
        }
    }

    fn slot(&mut self, which: Which) -> &mut Option<HotKey> {
        match which {
            Which::Raise => &mut self.raise,
            Which::TimeLapse => &mut self.timelapse,
        }
    }

    /// The hotkey whose keys were pressed, the raise first; a release is no press.
    fn which(self, event: GlobalHotKeyEvent) -> Option<Which> {
        if event.state != HotKeyState::Pressed {
            return None;
        }
        Which::ALL.into_iter().find(|&which| {
            self.get(which)
                .is_some_and(|hotkey| hotkey.id() == event.id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(text: &str) -> Binding {
        Binding::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    #[test]
    fn the_stored_form_round_trips_and_reads_nothing_else() {
        for which in Which::ALL {
            let binding = which.default_binding();
            assert_eq!(Binding::parse(&binding.to_setting()), Some(binding));
        }
        assert_eq!(Which::Raise.default_binding().to_setting(), "ctrl+alt+T");
        assert_eq!(
            Which::TimeLapse.default_binding().to_setting(),
            "ctrl+alt+H"
        );
        // Modifiers in any order, a key in either case; written back in the fixed order.
        assert_eq!(
            binding("super+shift+ctrl+f5").to_setting(),
            "ctrl+shift+super+F5"
        );
        for (code, name) in KEYS {
            let text = format!("ctrl+alt+{name}");
            assert_eq!(binding(&text).key, code);
            assert_eq!(binding(&text).to_setting(), text);
        }
        for bad in [
            "",
            "ctrl+alt+",
            "ctrl+alt+Tab",
            "ctrl+ctrl+T",
            "control+alt+T",
            "ctrl+alt+T+S",
            "+T",
            "meta+alt+T",
        ] {
            assert_eq!(Binding::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_binding_needs_two_modifiers_one_of_them_control_or_command() {
        let other = Which::TimeLapse.default_binding();
        assert_eq!(Which::Raise.default_binding().check(other), Ok(()));
        assert_eq!(binding("ctrl+super+1").check(other), Ok(()));
        assert_eq!(binding("super+shift+K").check(other), Ok(()));
        assert_eq!(binding("T").check(other), Err(Refusal::TooFewModifiers));
        assert_eq!(
            binding("ctrl+T").check(other),
            Err(Refusal::TooFewModifiers)
        );
        assert_eq!(
            binding("super+T").check(other),
            Err(Refusal::TooFewModifiers)
        );
        // macOS 15 refuses Option and Shift alone.
        assert_eq!(
            binding("alt+shift+T").check(other),
            Err(Refusal::OptionShift)
        );
        assert_eq!(binding("ctrl+alt+H").check(other), Err(Refusal::Taken));
    }

    #[test]
    fn the_registered_hotkey_has_the_binding_s_keys() {
        let hotkey = binding("ctrl+alt+shift+super+F12").hotkey();
        assert_eq!(hotkey.key, Code::F12);
        assert_eq!(
            hotkey.mods,
            Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::SUPER
        );
        assert_eq!(
            Which::Raise.default_binding().hotkey(),
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT)
        );
    }

    #[test]
    fn a_press_is_told_apart_by_its_keys_and_a_release_is_none() {
        let raise = Which::Raise.default_binding().hotkey();
        let timelapse = Which::TimeLapse.default_binding().hotkey();
        let event = |hotkey: HotKey, state| GlobalHotKeyEvent {
            id: hotkey.id(),
            state,
        };
        let bound = Bound {
            raise: Some(raise),
            timelapse: Some(timelapse),
        };
        assert_eq!(
            bound.which(event(raise, HotKeyState::Pressed)),
            Some(Which::Raise)
        );
        assert_eq!(
            bound.which(event(timelapse, HotKeyState::Pressed)),
            Some(Which::TimeLapse)
        );
        assert_eq!(bound.which(event(timelapse, HotKeyState::Released)), None);
        let other = binding("ctrl+super+F1").hotkey();
        assert_eq!(bound.which(event(other, HotKeyState::Pressed)), None);
        let unbound = Bound {
            timelapse: None,
            ..bound
        };
        assert_eq!(unbound.which(event(timelapse, HotKeyState::Pressed)), None);
    }
}
