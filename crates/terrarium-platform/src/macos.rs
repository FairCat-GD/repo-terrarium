//! macOS: the strip's window level, Spaces behaviour, placement and hover tracking, the
//! screens, and the notifications winit does not deliver.
//!
//! winit links objc2 0.5 and this module objc2 0.6, so the two never exchange types. The only
//! bridge is the `NSView` pointer in winit's raw window handle.

use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification,
    NSApplicationDidChangeScreenParametersNotification, NSApplicationDidResignActiveNotification,
    NSFloatingWindowLevel, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowLevel, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceScreensDidWakeNotification,
};
use objc2_core_foundation::{CFRetained, CFUUID};
use objc2_core_graphics::{CGWindowLevelForKey, CGWindowLevelKey};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSNumber, NSOperationQueue, NSPoint, NSProcessInfo,
    NSRect, NSSize, ns_string,
};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};

use crate::SystemEvent;
use crate::geometry::{Edge, Rect, strip_frame};

#[derive(Debug)]
pub enum PlatformError {
    /// `AppKit` windows may only be touched from the main thread.
    NotMainThread,
    /// The window handle is unavailable.
    Handle(HandleError),
    /// The handle is not an `AppKit` view.
    NotAppKit,
    /// The view is not in a window.
    NoWindow,
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMainThread => write!(f, "not on the main thread"),
            Self::Handle(err) => write!(f, "no window handle: {err}"),
            Self::NotAppKit => write!(f, "the window handle is not an AppKit view"),
            Self::NoWindow => write!(f, "the view is not in a window"),
        }
    }
}

impl std::error::Error for PlatformError {}

impl From<HandleError> for PlatformError {
    fn from(err: HandleError) -> Self {
        Self::Handle(err)
    }
}

/// The strip's `NSWindow`, retained. It can only exist on the main thread, and cannot leave it.
pub struct StripWindow {
    window: Retained<NSWindow>,
    view: Retained<NSView>,
}

impl StripWindow {
    /// Takes the window behind winit's view and makes it a strip: on every Space, fixed in
    /// Mission Control, out of the window cycle, not hidden by another app's Hide Others, and
    /// tracking the pointer while the app is inactive.
    pub fn attach(window: &impl HasWindowHandle) -> Result<Self, PlatformError> {
        let view = appkit_view(window)?;
        let window = view.window().ok_or(PlatformError::NoWindow)?;
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setCanHide(false);

        // winit's own tracking rect only reports the pointer while the app is active. This
        // area reports it always, to the same view, so winit's handlers turn it into
        // `CursorEntered`, `CursorMoved` and `CursorLeft`.
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: the owner is the view the area is added to, so it outlives the area (which
        // does not retain it), and it implements the mouse handlers the options ask for;
        // there is no user info.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                options,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);
        Ok(Self { window, view })
    }

    /// Just above the desktop icons: below every ordinary window, and above Finder's desktop
    /// window, which would otherwise compete for the strip's clicks.
    pub fn set_desktop_level(&self) {
        self.window.setLevel(desktop_icon_level() + 1);
    }

    /// Above ordinary windows, without activating the app.
    pub fn raise(&self) {
        self.window.setLevel(NSFloatingWindowLevel);
        self.window.orderFrontRegardless();
    }

    /// Shows the window without making it key, so the app is not activated.
    pub fn show(&self) {
        self.window.orderFrontRegardless();
    }

    pub fn level(&self) -> NSWindowLevel {
        self.window.level()
    }

    /// Moves the strip to its edge of `screen`, sized in the screen's physical pixels, and
    /// returns the frame it was given, in points.
    pub fn place(&self, screen: &Screen, edge: Edge, width_px: u32, height_px: u32) -> Rect {
        let frame = strip_frame(
            screen.frame,
            screen.visible_frame,
            edge,
            f64::from(width_px) / screen.scale,
            f64::from(height_px) / screen.scale,
        );
        self.window.setFrame_display(ns_rect(frame), true);
        frame
    }

    pub fn frame(&self) -> Rect {
        rect(self.window.frame())
    }

    /// Pops up `menu` at the pointer. It returns when the menu closes.
    pub fn show_context_menu(&self, menu: &muda::Menu) {
        use muda::ContextMenu;
        // SAFETY: `self.view` is a live, retained `NSView` inside a window, and a
        // `StripWindow` exists only on the main thread, where muda must be called.
        unsafe {
            menu.show_context_menu_for_nsview(Retained::as_ptr(&self.view).cast(), None);
        }
    }
}

fn appkit_view(window: &impl HasWindowHandle) -> Result<Retained<NSView>, PlatformError> {
    MainThreadMarker::new().ok_or(PlatformError::NotMainThread)?;
    let RawWindowHandle::AppKit(handle) = window.window_handle()?.as_raw() else {
        return Err(PlatformError::NotAppKit);
    };
    // SAFETY: raw-window-handle 0.6 guarantees that `ns_view` points to a valid `NSView` while
    // the handle is borrowed, and we are on the main thread. Retaining it keeps it valid after
    // the borrow ends.
    unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
        .ok_or(PlatformError::NoWindow)
}

fn desktop_icon_level() -> NSWindowLevel {
    // `CGWindowLevel` is an `i32` and `NSWindowLevel` an `isize`, so this widens.
    CGWindowLevelForKey(CGWindowLevelKey::DesktopIconWindowLevelKey) as NSWindowLevel
}

/// A display, as `AppKit` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// The `CGDirectDisplayID`. It can change when displays are reconnected.
    pub display_id: u32,
    /// The display's UUID, stable across reconnects and restarts.
    pub uuid: Option<String>,
    pub name: String,
    pub frame: Rect,
    /// The frame minus the menu bar and the Dock.
    pub visible_frame: Rect,
    /// Physical pixels per point.
    pub scale: f64,
    /// The screen with the menu bar, whose origin is the global origin.
    pub primary: bool,
}

impl Screen {
    /// The screen's width in physical pixels.
    pub fn physical_width(&self) -> u32 {
        physical(self.frame.width * self.scale)
    }

    pub fn physical_height(&self) -> u32 {
        physical(self.frame.height * self.scale)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn physical(pixels: f64) -> u32 {
    pixels.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

/// Every connected screen, the primary first. Empty off the main thread.
pub fn screens() -> Vec<Screen> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .filter_map(|(index, screen)| {
            let display_id = screen
                .deviceDescription()
                .objectForKey(ns_string!("NSScreenNumber"))?
                .downcast::<NSNumber>()
                .ok()?
                .unsignedIntValue();
            Some(Screen {
                display_id,
                uuid: display_uuid(display_id),
                name: screen.localizedName().to_string(),
                frame: rect(screen.frame()),
                visible_frame: rect(screen.visibleFrame()),
                scale: screen.backingScaleFactor(),
                primary: index == 0,
            })
        })
        .collect()
}

#[link(name = "ColorSync", kind = "framework")]
unsafe extern "C" {
    /// Returns a new `CFUUIDRef` the caller owns, or null when no display has this id.
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *mut CFUUID;
}

fn display_uuid(display_id: u32) -> Option<String> {
    // SAFETY: the function accepts any display id and returns null or a new UUID.
    let uuid = NonNull::new(unsafe { CGDisplayCreateUUIDFromDisplayID(display_id) })?;
    // SAFETY: the pointer is non-null and owned by us (the Create rule), so `CFRetained` takes
    // over that one reference and releases it when dropped.
    let uuid = unsafe { CFRetained::from_raw(uuid) };
    CFUUID::new_string(None, Some(&uuid)).map(|text| text.to_string())
}

/// Whether the app is the active one. False off the main thread.
pub fn app_is_active() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}

/// The name of the app the user is working in.
pub fn frontmost_app() -> Option<String> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .localizedName()
        .map(|name| name.to_string())
}

pub fn os_version() -> String {
    NSProcessInfo::processInfo()
        .operatingSystemVersionString()
        .to_string()
}

/// The "Displays have separate Spaces" setting. False off the main thread.
pub fn screens_have_separate_spaces() -> bool {
    MainThreadMarker::new().is_some_and(NSScreen::screensHaveSeparateSpaces)
}

/// What a notification centre returns for a block observer; it is needed to remove it.
type ObserverToken = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// The registered observers. Dropping this removes them.
pub struct Observers {
    registered: Vec<(Retained<NSNotificationCenter>, ObserverToken)>,
}

/// Calls `on_event` on the main thread for each [`SystemEvent`] until the result is dropped.
pub fn observe(on_event: impl Fn(SystemEvent) + Send + Sync + 'static) -> Observers {
    let on_event: Arc<dyn Fn(SystemEvent) + Send + Sync> = Arc::new(on_event);
    let queue = NSOperationQueue::mainQueue();
    let app = NSNotificationCenter::defaultCenter();
    // Workspace notifications are posted on the workspace's own centre, not the default one.
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: these are AppKit's exported notification-name constants, initialised before
    // `main` and never written.
    let subscriptions = unsafe {
        [
            (
                &app,
                NSApplicationDidChangeScreenParametersNotification,
                SystemEvent::ScreensChanged,
            ),
            (
                &app,
                NSApplicationDidBecomeActiveNotification,
                SystemEvent::AppBecameActive,
            ),
            (
                &app,
                NSApplicationDidResignActiveNotification,
                SystemEvent::AppResignedActive,
            ),
            (
                &workspace,
                NSWorkspaceDidWakeNotification,
                SystemEvent::Woke,
            ),
            (
                &workspace,
                NSWorkspaceScreensDidWakeNotification,
                SystemEvent::ScreensWoke,
            ),
            (
                &workspace,
                NSWorkspaceActiveSpaceDidChangeNotification,
                SystemEvent::SpaceChanged,
            ),
        ]
    };
    let registered = subscriptions
        .into_iter()
        .map(|(center, name, event)| {
            let on_event = Arc::clone(&on_event);
            let block = RcBlock::new(move |_: NonNull<NSNotification>| on_event(event));
            // SAFETY: there is no object filter; the main queue runs the block on the main
            // thread; the block captures only `Send + Sync` state, so it may be sent.
            let token = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    None,
                    Some(&queue),
                    &block,
                )
            };
            (center.clone(), token)
        })
        .collect();
    Observers { registered }
}

impl Drop for Observers {
    fn drop(&mut self) {
        for (center, token) in self.registered.drain(..) {
            // SAFETY: `token` is the observer this centre returned when the block was added,
            // and it is removed exactly once.
            unsafe { center.removeObserver(token.as_ref()) };
        }
    }
}

fn rect(r: NSRect) -> Rect {
    Rect {
        x: r.origin.x,
        y: r.origin.y,
        width: r.size.width,
        height: r.size.height,
    }
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.width, r.height))
}
