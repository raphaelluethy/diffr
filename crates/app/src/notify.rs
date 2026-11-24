//! Desktop notifications: the UserNotifications framework on macOS, so they
//! carry diffr's name and icon; freedesktop's (notify-send) on Linux.

use std::sync::OnceLock;

use gpui::App;

use crate::window::Diffr;

/// The repositories of the notifications clicked, from whichever thread the
/// system reports a click on, to the main thread.
fn clicks() -> &'static (async_channel::Sender<String>, async_channel::Receiver<String>) {
    static CLICKS: OnceLock<(async_channel::Sender<String>, async_channel::Receiver<String>)> = OnceLock::new();
    CLICKS.get_or_init(async_channel::unbounded)
}

/// Takes the clicks on notifications: each brings up the window of the
/// repository it is about. Called once, as the app starts, so a click on a
/// notification from an earlier run is also taken.
pub fn listen(cx: &mut App) {
    #[cfg(target_os = "macos")]
    mac::listen();
    let rx = clicks().1.clone();
    cx.spawn(async move |cx| {
        while let Ok(root) = rx.recv().await {
            cx.update(|cx| {
                let window = cx.windows().into_iter().find(|w| {
                    w.downcast::<Diffr>().and_then(|h| h.read_with(cx, |this, _| this.root == root).ok()).unwrap_or(false)
                });
                if let Some(w) = window.or_else(|| cx.windows().first().copied()) {
                    w.update(cx, |_, window, _| window.activate_window()).ok();
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}

/// Posts a notification; `root` is the repository it is about, whose window
/// a click on it brings up.
pub fn notify(title: &str, body: &str, root: Option<&str>) {
    #[cfg(target_os = "macos")]
    if mac::notify(title, body, root.unwrap_or_default()) {
        return;
    }
    let _ = root;
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = if cfg!(target_os = "macos") {
            // Outside an app bundle (cargo run) the framework is unavailable,
            // so the notification comes from Script Editor.
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!("display notification {:?} with title {:?}", body, title))
                .output()
        } else {
            std::process::Command::new("notify-send")
                .args(["--app-name=Diffr", "--hint=string:desktop-entry:dev.diffr", &title, &body])
                .output()
        };
    });
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::atomic::{AtomicU64, Ordering};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::{AnyThread, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    /// The center refuses to work, raising an exception that aborts the
    /// process, outside an app bundle: as `cargo run` runs diffr.
    fn bundled() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    /// The repository a notification is about rides in its identifier.
    const SEP: char = '\u{1f}';

    pub fn listen() {
        if !bundled() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = Delegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly; this one serves the whole run.
        std::mem::forget(delegate);
    }

    /// Posts through the notification center; false outside an app bundle.
    pub fn notify(title: &str, body: &str, root: &str) -> bool {
        if !bundled() {
            return false;
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!("{}.{}{SEP}{root}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));

        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);

        // Once the user has answered, asking returns their answer at once;
        // the first time, the notification waits for it.
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let poster = center.clone();
        let then = RcBlock::new(move |granted: Bool, _: *mut NSError| {
            if granted.as_bool() {
                poster.addNotificationRequest_withCompletionHandler(&request, None);
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &then,
        );
        true
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Delegate has
        // no Drop.
        #[unsafe(super(NSObject))]
        #[ivars = ()]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// A click on a notification, on any thread.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                done: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                let root = id.split_once(SEP).map(|(_, r)| r.to_string()).unwrap_or_default();
                let _ = super::clicks().0.try_send(root);
                done.call(());
            }

            /// Without it macOS holds back the banner while diffr is
            /// frontmost; diffr notifies only when its window is not active.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                done.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's init is its designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }
}
