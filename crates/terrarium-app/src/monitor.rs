//! Which monitor the strip goes on.

use serde::Serialize;

/// A monitor identity that survives reconnects and restarts: the display UUID on macOS.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MonitorId(pub String);

/// What the choice needs to know about a monitor, and what the probe reports about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonitorInfo {
    pub id: MonitorId,
    pub name: String,
    pub physical_width: u32,
    pub physical_height: u32,
    /// Physical pixels per point.
    pub scale: f64,
    pub primary: bool,
    /// Platform detail for the probe report, such as the frames in points.
    pub detail: String,
}

/// The monitor from the setting if it is connected; otherwise a monitor other than the primary,
/// so the strip lands on a second screen; otherwise the primary. Among several non-primary
/// monitors the lowest id wins, so the choice is the same on every launch.
pub fn choose(monitors: &[MonitorInfo], preferred: Option<&MonitorId>) -> Option<usize> {
    if let Some(preferred) = preferred
        && let Some(index) = monitors.iter().position(|m| &m.id == preferred)
    {
        return Some(index);
    }
    monitors
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.primary)
        .min_by(|(_, a), (_, b)| a.id.cmp(&b.id))
        .map(|(index, _)| index)
        .or_else(|| monitors.iter().position(|m| m.primary))
        .or_else(|| (!monitors.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, primary: bool) -> MonitorInfo {
        MonitorInfo {
            id: MonitorId(id.to_owned()),
            name: format!("Display {id}"),
            physical_width: 3840,
            physical_height: 2160,
            scale: 2.0,
            primary,
            detail: String::new(),
        }
    }

    #[test]
    fn a_connected_preferred_monitor_wins() {
        let monitors = [monitor("B", true), monitor("C", false)];
        let preferred = MonitorId("B".to_owned());
        assert_eq!(choose(&monitors, Some(&preferred)), Some(0));
    }

    #[test]
    fn a_disconnected_preferred_monitor_falls_back_to_a_non_primary_one() {
        let monitors = [monitor("B", true), monitor("C", false)];
        let preferred = MonitorId("gone".to_owned());
        assert_eq!(choose(&monitors, Some(&preferred)), Some(1));
    }

    #[test]
    fn several_non_primary_monitors_pick_the_lowest_id() {
        let monitors = [monitor("A", true), monitor("Z", false), monitor("M", false)];
        assert_eq!(choose(&monitors, None), Some(2));
    }

    #[test]
    fn a_lone_primary_is_chosen() {
        assert_eq!(choose(&[monitor("A", true)], None), Some(0));
    }

    #[test]
    fn with_no_primary_reported_the_first_monitor_is_chosen() {
        assert_eq!(choose(&[monitor("A", false)], None), Some(0));
    }

    #[test]
    fn no_monitor_means_no_choice() {
        assert_eq!(choose(&[], None), None);
    }
}
