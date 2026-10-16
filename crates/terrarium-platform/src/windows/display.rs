//! The display configuration's paths: each source's GDI name with its monitor's device path and
//! friendly name, which `crate::display` turns into ids that survive restarts.

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

use super::struct_size;
use crate::display::DisplayPath;

/// How often the paths are asked for again when displays changed between sizing the buffers
/// and filling them.
const TRIES: usize = 3;

/// The active paths, or none when the configuration cannot be read.
pub(super) fn paths() -> Vec<DisplayPath> {
    let Some(paths) = active_paths() else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(|path| {
            let gdi = source_name(path)?;
            let (device_path, friendly) = target_name(path).unwrap_or_default();
            Some(DisplayPath {
                gdi,
                device_path,
                friendly,
            })
        })
        .collect()
}

fn active_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..TRIES {
        let (mut path_count, mut mode_count) = (0_u32, 0_u32);
        // SAFETY: both counts are writable `u32`s that outlive the call.
        let sized = unsafe {
            GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                &raw mut mode_count,
            )
        };
        if sized != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); usize::try_from(path_count).ok()?];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); usize::try_from(mode_count).ok()?];
        // SAFETY: each buffer holds as many elements as its count says, and the call writes no
        // more than that, lowering the counts to what it wrote.
        let filled = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                paths.as_mut_ptr(),
                &raw mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if filled == ERROR_SUCCESS {
            paths.truncate(usize::try_from(path_count).ok()?);
            return Some(paths);
        }
        if filled != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
    }
    None
}

/// The path's source's GDI name (`\\.\DISPLAY1`).
fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: struct_size::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..DISPLAYCONFIG_SOURCE_DEVICE_NAME::default()
    };
    // SAFETY: the header starts a whole `DISPLAYCONFIG_SOURCE_DEVICE_NAME` whose size it
    // states, which outlives the call; the call writes only within that size.
    let asked = unsafe { DisplayConfigGetDeviceInfo(&raw mut name.header) };
    (asked == 0).then(|| text(&name.viewGdiDeviceName))
}

/// The path's target's monitor device path and friendly name.
fn target_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<(String, String)> {
    let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: struct_size::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..DISPLAYCONFIG_TARGET_DEVICE_NAME::default()
    };
    // SAFETY: as for the source's name, with a whole `DISPLAYCONFIG_TARGET_DEVICE_NAME`.
    let asked = unsafe { DisplayConfigGetDeviceInfo(&raw mut name.header) };
    (asked == 0).then(|| {
        (
            text(&name.monitorDevicePath),
            text(&name.monitorFriendlyDeviceName),
        )
    })
}

/// A NUL-terminated UTF-16 buffer as text.
fn text(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}
