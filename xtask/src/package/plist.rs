//! The Mac app's `Info.plist`.
//!
//! - **The bundle id** is the one the login item asks `SMAppService` about, so open at login
//!   finds the installed app (`terrarium-platform`'s `login`).
//! - **`LSUIElement`:** no Dock icon, from the first moment; the app sets the same
//!   accessory policy itself, a moment later.
//! - **The folder reasons:** macOS shows the reason under its own line when the app first reads a
//!   repository in a folder it guards.

use std::fmt::Write as _;

use terrarium_store::identifiers::{BINARY, BUNDLE_ID};

use super::words::Words;
use crate::release::MACOS_MINIMUM;

/// The icon's file in the bundle's Resources, without its extension.
pub const ICON_FILE: &str = "AppIcon";

/// The folders macOS asks about, by the key that holds the reason for each.
const FOLDER_KEYS: [&str; 5] = [
    "NSDesktopFolderUsageDescription",
    "NSDocumentsFolderUsageDescription",
    "NSDownloadsFolderUsageDescription",
    "NSNetworkVolumesUsageDescription",
    "NSRemovableVolumesUsageDescription",
];

enum Value<'a> {
    Text(&'a str),
    True,
}

/// The `Info.plist` for `version` with `words`, its keys in order.
pub fn info_plist(words: &Words, version: &str) -> String {
    let mut entries: Vec<(&str, Value)> = vec![
        ("CFBundleDevelopmentRegion", Value::Text("en")),
        ("CFBundleDisplayName", Value::Text(&words.product)),
        ("CFBundleExecutable", Value::Text(BINARY)),
        ("CFBundleIconFile", Value::Text(ICON_FILE)),
        ("CFBundleIdentifier", Value::Text(BUNDLE_ID)),
        ("CFBundleInfoDictionaryVersion", Value::Text("6.0")),
        ("CFBundleName", Value::Text(&words.product)),
        ("CFBundlePackageType", Value::Text("APPL")),
        ("CFBundleShortVersionString", Value::Text(version)),
        ("CFBundleVersion", Value::Text(version)),
        (
            "LSApplicationCategoryType",
            Value::Text("public.app-category.developer-tools"),
        ),
        ("LSMinimumSystemVersion", Value::Text(MACOS_MINIMUM)),
        ("LSUIElement", Value::True),
        ("NSHighResolutionCapable", Value::True),
        ("NSHumanReadableCopyright", Value::Text(&words.licence)),
        ("NSSupportsAutomaticGraphicsSwitching", Value::True),
    ];
    entries.extend(
        FOLDER_KEYS
            .iter()
            .map(|key| (*key, Value::Text(&words.folder_reason))),
    );
    entries.sort_by_key(|(key, _)| *key);
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries {
        let _ = writeln!(out, "\t<key>{}</key>", escape(key));
        let _ = match value {
            Value::Text(text) => writeln!(out, "\t<string>{}</string>", escape(text)),
            Value::True => writeln!(out, "\t<true/>"),
        };
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::words::tests::words;

    #[test]
    fn the_plist_names_the_bundle_its_version_and_why_it_reads_folders() {
        let plist = info_plist(&words(), "0.31.0");
        let has =
            |key: &str, value: &str| plist.contains(&format!("\t<key>{key}</key>\n\t{value}\n"));
        assert!(has(
            "CFBundleIdentifier",
            "<string>io.github.faircat-gd.repo-terrarium</string>"
        ));
        assert!(has("CFBundleExecutable", "<string>repo-terrarium</string>"));
        assert!(has("CFBundleName", "<string>Moss Box</string>"));
        assert!(has("CFBundleShortVersionString", "<string>0.31.0</string>"));
        assert!(has("LSMinimumSystemVersion", "<string>13.0</string>"));
        assert!(has("LSUIElement", "<true/>"));
        assert!(has(
            "NSHumanReadableCopyright",
            "<string>Code MIT; art CC0</string>"
        ));
        assert!(has("NSSupportsAutomaticGraphicsSwitching", "<true/>"));
        for key in FOLDER_KEYS {
            assert!(
                has(key, "<string>Moss Box reads what you connected.</string>"),
                "{key}"
            );
        }
        assert!(plist.starts_with("<?xml"));
        assert!(plist.ends_with("</dict>\n</plist>\n"));
        // Sorted, so the same words always write the same file.
        let keys: Vec<&str> = plist
            .lines()
            .filter_map(|line| line.trim().strip_prefix("<key>"))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
        assert_eq!(escape("a & <b>"), "a &amp; &lt;b&gt;");
    }
}
