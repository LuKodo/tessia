//! The macOS menu bar. tao has no menu, and without an Edit menu Cmd+C, Cmd+V
//! and Cmd+A never reach the webview's text fields (Wails adds the same menu by
//! default). Other platforms need nothing.

#[cfg(target_os = "macos")]
pub type Menu = muda::Menu;
#[cfg(not(target_os = "macos"))]
pub type Menu = ();

/// Installs the menu bar; the returned handle must stay alive while the app runs.
#[cfg(target_os = "macos")]
pub fn install() -> Option<Menu> {
    match build() {
        Ok(menu) => {
            menu.init_for_nsapp();
            Some(menu)
        }
        Err(error) => {
            tracing::warn!(%error, "could not build the menu bar");
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() -> Option<Menu> {
    None
}

#[cfg(target_os = "macos")]
fn build() -> muda::Result<muda::Menu> {
    use muda::{Menu, PredefinedMenuItem as Item, Submenu};

    let separator = Item::separator();
    let app = Submenu::with_items(
        super::TITLE,
        true,
        &[
            &Item::about(Some("Acerca de Tessia"), None),
            &separator,
            &Item::services(Some("Servicios")),
            &Item::separator(),
            &Item::hide(Some("Ocultar Tessia")),
            &Item::hide_others(Some("Ocultar otros")),
            &Item::show_all(Some("Mostrar todo")),
            &Item::separator(),
            &Item::quit(Some("Salir de Tessia")),
        ],
    )?;
    let edit = Submenu::with_items(
        "Edición",
        true,
        &[
            &Item::undo(Some("Deshacer")),
            &Item::redo(Some("Rehacer")),
            &Item::separator(),
            &Item::cut(Some("Cortar")),
            &Item::copy(Some("Copiar")),
            &Item::paste(Some("Pegar")),
            &Item::select_all(Some("Seleccionar todo")),
        ],
    )?;
    let window = Submenu::with_items(
        "Ventana",
        true,
        &[
            &Item::minimize(Some("Minimizar")),
            &Item::close_window(Some("Cerrar ventana")),
        ],
    )?;
    Menu::with_items(&[&app, &edit, &window])
}
