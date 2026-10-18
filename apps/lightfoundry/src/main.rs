//! LightFoundry desktop app: hosts the egui front end in an eframe window.

mod host;
mod net;

use std::path::PathBuf;

use lightfoundry_engine::Session;
use lightfoundry_ui_egui::LightFoundryApp;
use serde_json::json;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut session = Session::new();
    // Agent access is off unless asked for: it lets a local program edit the open drawing.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let agent_access = args.iter().any(|a| a == "--agent-link") || std::env::var_os("LIGHTFOUNDRY_AGENT_LINK").is_some_and(|v| v == "1");
    if let Some(path) = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from) {
        let (id, key) = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("svg") => ("file.import_svg", "path"),
            Some("dxf") => ("file.import_dxf", "path"),
            Some("png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp") => ("file.import_image", "path"),
            _ => ("file.open", "path"),
        };
        if let Err(e) = session.execute(id, &json!({ key: path })) {
            log::error!("could not open {}: {e}", path.display());
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_min_inner_size([900.0, 560.0]).with_title("LightFoundry"),
        ..Default::default()
    };
    host::make_data_folder_private();
    eframe::run_native(
        "LightFoundry",
        options,
        Box::new(move |cc| {
            let agent_link = agent_access.then(|| start_agent_link(&cc.egui_ctx)).flatten();
            Ok(Box::new(host::Host { app: LightFoundryApp::new(session, Box::new(host::DesktopPlatform::default())), agent_link }))
        }),
    )
}

/// Opens the agent link, telling the person where it is. A failure leaves the app running without it.
fn start_agent_link(ctx: &egui::Context) -> Option<lightfoundry_mcp::link::LinkServer> {
    let Some(file) = lightfoundry_mcp::link::default_link_file() else {
        log::error!("agent access: no folder for the link file");
        return None;
    };
    let ctx = ctx.clone();
    match lightfoundry_mcp::link::LinkServer::start(&file, move || ctx.request_repaint()) {
        Ok(link) => {
            log::warn!("agent access is ON: programs run by this user can edit the open drawing (link file {})", file.display());
            Some(link)
        }
        Err(e) => {
            log::error!("agent access could not start: {e}");
            None
        }
    }
}
