#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Punto de entrada del binario nativo.
//! Toda la logica vive en `lib.rs` para que el crate tambien pueda
//! compilarse como libreria (necesario para las pruebas de integracion).

#[cfg(windows)]
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
}

fn main() {
    // El binario no lleva manifiesto de aplicacion, asi que Windows arranca el
    // proceso como DPI-unaware: la ventana se crea con tamano fisico mientras
    // WebView2 renderiza al 125/150 %, de modo que el contenido queda mas grande
    // que su ventana y se recorta. Declarar Per-Monitor-V2 antes de crear
    // cualquier ventana alinea ambas metricas.
    #[cfg(windows)]
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    git_agent_desktop_lib::run()
}
