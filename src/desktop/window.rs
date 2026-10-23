//! The tao window and the wry webview.
//!
//! Window: title `Tessia`, 1280x800, minimum 960x600, system theme, no custom
//! chrome (what Go's Wails shell sets). The page is served by the custom
//! protocol of [`protocol`]; navigation away from the app origin, new windows
//! and permission prompts are refused, and devtools exist only in debug builds.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::{
    NewWindowResponse, PermissionResponse, WebContext, WebView, WebViewBuilder,
    http::Response as HttpResponse,
};

use super::downloads::{DownloadOutcome, Downloads, downloads_dir};
use super::protocol::{self, Bridge};
use super::{Backend, TITLE};

const WINDOW_SIZE: (f64, f64) = (1280.0, 800.0);
const MIN_WINDOW_SIZE: (f64, f64) = (960.0, 600.0);

/// How long a download announcement stays in the window title.
const ANNOUNCEMENT: Duration = Duration::from_secs(6);

/// Messages the webview callbacks post to the event loop.
#[derive(Debug)]
enum AppEvent {
    Download(DownloadOutcome),
}

/// Opens the window and runs the event loop until it is closed, then releases
/// the backend. `EventLoop::run` never returns: the process ends after the
/// window closes, so everything that must be released is released in the loop.
pub(super) fn run(backend: Backend) -> Result<()> {
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(TITLE)
        .with_inner_size(LogicalSize::new(WINDOW_SIZE.0, WINDOW_SIZE.1))
        .with_min_inner_size(LogicalSize::new(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1))
        .build(&event_loop)
        .context("no se pudo crear la ventana")?;
    let menu = super::menu::install();
    let mut web_context = WebContext::new(webview_data_dir());
    let webview = build_webview(&window, &mut web_context, backend.bridge(), proxy)?;

    let mut shell = Shell {
        backend: Some(backend),
        webview: Some(webview),
        _web_context: web_context,
        _menu: menu,
        announced_until: None,
    };
    event_loop.run(move |event, _, flow| {
        shell.expire_announcement(&window);
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                window.set_visible(false);
                shell.release();
                *flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => shell.release(),
            Event::UserEvent(AppEvent::Download(outcome)) => shell.announce(&window, &outcome),
            _ => {}
        }
        if *flow != ControlFlow::Exit {
            *flow = match shell.announced_until {
                Some(until) => ControlFlow::WaitUntil(until),
                None => ControlFlow::Wait,
            };
        }
    })
}

/// What the event loop owns until the window closes.
struct Shell {
    backend: Option<Backend>,
    webview: Option<WebView>,
    _web_context: WebContext,
    _menu: Option<super::menu::Menu>,
    announced_until: Option<Instant>,
}

impl Shell {
    /// Stops the API, drops the webview and releases the stores. Idempotent
    /// (close request, then loop destruction).
    fn release(&mut self) {
        if let Some(backend) = self.backend.take() {
            backend.close();
        }
        self.webview.take();
    }

    /// Shows a finished download in the window title for a few seconds.
    fn announce(&mut self, window: &Window, outcome: &DownloadOutcome) {
        window.set_title(&announcement(outcome));
        self.announced_until = Some(Instant::now() + ANNOUNCEMENT);
    }

    fn expire_announcement(&mut self, window: &Window) {
        if self.announced_until.is_some_and(|t| Instant::now() >= t) {
            window.set_title(TITLE);
            self.announced_until = None;
        }
    }
}

fn announcement(outcome: &DownloadOutcome) -> String {
    match (&outcome.file_name, outcome.success) {
        (Some(name), true) => format!("{TITLE} · Descarga completada: {name} (carpeta Descargas)"),
        (_, true) => format!("{TITLE} · Descarga completada (carpeta Descargas)"),
        (_, false) => format!("{TITLE} · No se pudo completar la descarga"),
    }
}

/// Where the webview keeps its profile (localStorage holds the theme). Only
/// Windows and Linux honor it; WebView2 would otherwise write next to the
/// executable, which may be a read-only folder.
fn webview_data_dir() -> Option<std::path::PathBuf> {
    let dir = dirs::data_local_dir()?.join(TITLE).join("webview");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn build_webview(
    window: &Window,
    web_context: &mut WebContext,
    bridge: Arc<Bridge>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<WebView> {
    let origin = protocol::app_origin();
    let downloads = Rc::new(RefCell::new(Downloads::new(origin, downloads_dir())));
    let on_started = Rc::clone(&downloads);

    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_asynchronous_custom_protocol(
            protocol::SCHEME.to_string(),
            move |_id, request, responder| {
                bridge.handle(
                    request,
                    Box::new(move |response: HttpResponse<_>| responder.respond(response)),
                );
            },
        )
        .with_url(protocol::start_url())
        .with_devtools(cfg!(debug_assertions))
        .with_navigation_handler(move |url| protocol::is_app_url(&url, origin))
        .with_new_window_req_handler(|_url, _features| NewWindowResponse::Deny)
        .with_permission_handler(|_kind| PermissionResponse::Deny)
        .with_download_started_handler(move |url, destination| {
            on_started.borrow_mut().started(&url, destination)
        })
        .with_download_completed_handler(move |url, path, success| {
            let outcome = downloads.borrow_mut().completed(&url, path, success);
            let _ = proxy.send_event(AppEvent::Download(outcome));
        });
    attach(builder, window)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn attach(builder: WebViewBuilder<'_>, window: &Window) -> Result<WebView> {
    builder
        .build(window)
        .context("no se pudo abrir el visor web")
}

/// On Linux the webview lives in the window's default GTK box (works on X11 and
/// Wayland, unlike `build`).
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn attach(builder: WebViewBuilder<'_>, window: &Window) -> Result<WebView> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;

    let vbox = window
        .default_vbox()
        .context("la ventana no tiene contenedor GTK")?;
    builder
        .build_gtk(vbox)
        .context("no se pudo abrir el visor web")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcements_are_in_spanish_and_name_the_file() {
        let done = DownloadOutcome {
            file_name: Some("informe.csv".into()),
            success: true,
        };
        assert_eq!(
            announcement(&done),
            "Tessia · Descarga completada: informe.csv (carpeta Descargas)"
        );
        let failed = DownloadOutcome {
            file_name: None,
            success: false,
        };
        assert_eq!(
            announcement(&failed),
            "Tessia · No se pudo completar la descarga"
        );
    }
}
