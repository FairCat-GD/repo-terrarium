//! A Windows display's identity across restarts and reconnections. Its GDI name
//! (`\\.\DISPLAY1`) is renumbered as displays come and go; the monitor's device path from
//! `QueryDisplayConfig` names the monitor on its connector, and stays. Pure, so it is tested on
//! every OS; `windows::display_paths` reads the paths.

use std::collections::BTreeMap;

/// One display path as `QueryDisplayConfig` reports it: the source's GDI name, and the
/// target's monitor device path and friendly name, either of which can be empty (a remote or
/// indirect display, an internal panel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPath {
    pub gdi: String,
    pub device_path: String,
    pub friendly: String,
}

/// What a display is called by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableName {
    /// The monitor's device path, or the GDI name when it has none.
    pub id: String,
    /// The monitor's own name ("ALDR A2700Q"), when it gives one.
    pub friendly: Option<String>,
    /// The id is the device path, which survives restarts.
    pub from_display_config: bool,
}

/// Each GDI name's stable name. A source shown on several monitors (a clone) takes the
/// smallest device path, so the choice is the same every time.
pub fn stable_names(paths: &[DisplayPath]) -> BTreeMap<String, StableName> {
    let mut names: BTreeMap<String, StableName> = BTreeMap::new();
    for path in paths.iter().filter(|path| !path.gdi.is_empty()) {
        let friendly = (!path.friendly.trim().is_empty()).then(|| path.friendly.trim().to_owned());
        let candidate = if path.device_path.is_empty() {
            StableName {
                id: path.gdi.clone(),
                friendly,
                from_display_config: false,
            }
        } else {
            StableName {
                id: path.device_path.clone(),
                friendly,
                from_display_config: true,
            }
        };
        names
            .entry(path.gdi.clone())
            .and_modify(|held| {
                let better = match (held.from_display_config, candidate.from_display_config) {
                    (false, true) => true,
                    (true, true) => candidate.id < held.id,
                    _ => false,
                };
                if better {
                    *held = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    names
}

/// The stable name of the display with GDI name `gdi`: from `names`, or the GDI name itself
/// when the display configuration did not list it.
pub fn stable_name(gdi: &str, names: &BTreeMap<String, StableName>) -> StableName {
    names.get(gdi).cloned().unwrap_or_else(|| StableName {
        id: gdi.to_owned(),
        friendly: None,
        from_display_config: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(gdi: &str, device_path: &str, friendly: &str) -> DisplayPath {
        DisplayPath {
            gdi: gdi.to_owned(),
            device_path: device_path.to_owned(),
            friendly: friendly.to_owned(),
        }
    }

    const ONE: &str = r"\\.\DISPLAY1";
    const TWO: &str = r"\\.\DISPLAY2";
    const ALDR: &str =
        r"\\?\DISPLAY#ACM40F4#5&2a1f7b8c&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    const MO: &str =
        r"\\?\DISPLAY#MOB5B09#5&2a1f7b8c&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";

    #[test]
    fn each_gdi_name_takes_its_monitors_device_path() {
        let names = stable_names(&[path(ONE, ALDR, "ALDR A2700Q"), path(TWO, MO, "MO HDR 4K")]);
        assert_eq!(
            names[ONE],
            StableName {
                id: ALDR.to_owned(),
                friendly: Some("ALDR A2700Q".to_owned()),
                from_display_config: true,
            }
        );
        assert_eq!(names[TWO].id, MO);
    }

    #[test]
    fn a_cloned_source_takes_the_smallest_device_path() {
        for paths in [
            [path(ONE, MO, "MO HDR 4K"), path(ONE, ALDR, "ALDR A2700Q")],
            [path(ONE, ALDR, "ALDR A2700Q"), path(ONE, MO, "MO HDR 4K")],
        ] {
            let names = stable_names(&paths);
            assert_eq!(names[ONE].id, ALDR);
            assert_eq!(names[ONE].friendly.as_deref(), Some("ALDR A2700Q"));
        }
    }

    #[test]
    fn a_display_without_a_device_path_is_called_by_its_gdi_name() {
        let names = stable_names(&[path(ONE, "", "")]);
        assert_eq!(
            names[ONE],
            StableName {
                id: ONE.to_owned(),
                friendly: None,
                from_display_config: false,
            }
        );
        // A path found for it wins over one without.
        let names = stable_names(&[path(ONE, "", ""), path(ONE, ALDR, "")]);
        assert_eq!(names[ONE].id, ALDR);
        // One the configuration did not list at all.
        assert_eq!(stable_name(TWO, &names).id, TWO);
        assert!(!stable_name(TWO, &names).from_display_config);
    }

    #[test]
    fn an_empty_friendly_name_is_none_and_a_nameless_source_is_skipped() {
        let names = stable_names(&[path(ONE, ALDR, "  "), path("", MO, "MO HDR 4K")]);
        assert_eq!(names[ONE].friendly, None);
        assert_eq!(names.len(), 1);
    }
}
