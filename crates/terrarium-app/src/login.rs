//! Opening at login: the first run's box, pre-checked, applied once the repository is
//! connected, and never offered again; settings carry its toggle. The OS is reached through
//! [`LoginItem`], so what the first run asks of it is tested without touching it.

use terrarium_platform::login::{self, LoginError};
use terrarium_store::identifiers::BUNDLE_ID;

use crate::copy;

/// The OS's login items, as far as the app uses them.
pub trait LoginItem {
    fn set(&mut self, on: bool) -> Result<(), LoginError>;
}

/// This app, as the OS knows it by its bundle id.
pub struct System;

impl LoginItem for System {
    fn set(&mut self, on: bool) -> Result<(), LoginError> {
        login::set(BUNDLE_ID, on)
    }
}

/// The first run's choice, `wanted`, applied: the OS is asked only when the box was checked,
/// as nothing registered the app before. The line to print when it could not be applied, as
/// under `cargo run`, which runs no app bundle; the setting keeps what was chosen either way.
pub fn first_run(item: &mut dyn LoginItem, wanted: bool) -> Option<String> {
    if !wanted {
        return None;
    }
    item.set(true).err().map(|err| copy::login_not_set(&err))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A login item that records what it is asked, and answers `answer`.
    struct Mock {
        asked: Vec<bool>,
        answer: Result<(), LoginError>,
    }

    impl LoginItem for Mock {
        fn set(&mut self, on: bool) -> Result<(), LoginError> {
            self.asked.push(on);
            self.answer.clone()
        }
    }

    fn mock(answer: Result<(), LoginError>) -> Mock {
        Mock {
            asked: Vec::new(),
            answer,
        }
    }

    #[test]
    fn a_checked_box_registers_the_app_and_an_unchecked_one_asks_nothing() {
        let mut item = mock(Ok(()));
        assert_eq!(first_run(&mut item, true), None);
        assert_eq!(item.asked, [true]);
        let mut item = mock(Ok(()));
        assert_eq!(first_run(&mut item, false), None);
        assert!(item.asked.is_empty());
    }

    #[test]
    fn a_build_that_is_no_app_bundle_says_so_and_a_refusal_gives_the_os_s_reason() {
        let mut item = mock(Err(LoginError::NotBundled));
        let line = first_run(&mut item, true).expect("a line");
        assert!(line.contains("installed app"), "{line}");
        let mut item = mock(Err(LoginError::Os("not permitted".into())));
        let line = first_run(&mut item, true).expect("a line");
        assert!(line.ends_with("not permitted"), "{line}");
    }
}
