//! Reading a built Mac binary: the oldest macOS it says it runs on. The linker writes it from
//! `MACOSX_DEPLOYMENT_TARGET`, and rustc's default is older than the app's 13.

/// `LC_BUILD_VERSION`, and the older `LC_VERSION_MIN_MACOSX`.
const LC_BUILD_VERSION: u32 = 0x32;
const LC_VERSION_MIN_MACOSX: u32 = 0x24;

/// The minimum macOS version a thin 64-bit Mach-O image names, as (major, minor, patch).
pub fn minimum_macos(bytes: &[u8]) -> Option<(u32, u32, u32)> {
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if u32_at(0)? != 0xfeed_facf {
        return None;
    }
    let commands = u32_at(16)?;
    let mut at = 32;
    for _ in 0..commands {
        let command = u32_at(at)?;
        let size = usize::try_from(u32_at(at + 4)?).ok()?;
        // `LC_BUILD_VERSION` holds the platform, then the version; the older command holds the
        // version first. Either packs it as xxxx.yy.zz in nibbles.
        let packed = match command {
            LC_BUILD_VERSION => Some(u32_at(at + 12)?),
            LC_VERSION_MIN_MACOSX => Some(u32_at(at + 8)?),
            _ => None,
        };
        if let Some(packed) = packed {
            return Some((packed >> 16, (packed >> 8) & 0xff, packed & 0xff));
        }
        if size == 0 {
            return None;
        }
        at += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thin 64-bit Mach-O header with one load command of `command`, its version packed.
    fn image(command: u32, packed: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0xfeed_facf_u32.to_le_bytes());
        out.extend(0x0100_000c_u32.to_le_bytes()); // arm64
        out.extend([0_u8; 8]);
        out.extend(1_u32.to_le_bytes()); // one command
        out.extend(24_u32.to_le_bytes());
        out.extend([0_u8; 8]);
        out.extend(command.to_le_bytes());
        out.extend(24_u32.to_le_bytes());
        if command == LC_BUILD_VERSION {
            out.extend(1_u32.to_le_bytes()); // macOS
            out.extend(packed.to_le_bytes());
        } else {
            out.extend(packed.to_le_bytes());
            out.extend(0_u32.to_le_bytes());
        }
        out.extend([0_u8; 8]);
        out
    }

    #[test]
    fn the_minimum_version_is_read_from_either_command() {
        assert_eq!(
            minimum_macos(&image(LC_BUILD_VERSION, 0x000d_0000)),
            Some((13, 0, 0))
        );
        assert_eq!(
            minimum_macos(&image(LC_VERSION_MIN_MACOSX, 0x000b_0201)),
            Some((11, 2, 1))
        );
        assert_eq!(minimum_macos(b"MZ"), None);
        assert_eq!(minimum_macos(&image(0x19, 0)), None);
    }
}
