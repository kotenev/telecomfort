mod ai;
mod config;
mod db;
mod integrations;
mod media;
mod sync;
mod telegram;
mod ui;
mod utils;

use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "telecomfort=info".into()),
        )
        .init();

    let cfg = config::AppConfig::load().unwrap_or_default();
    let db = Arc::new(db::Db::open()?);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let rt_handle = rt.handle().clone();
    // Держим рантайм живым в отдельном потоке: eframe крутит свой event-loop,
    // а tokio-задачи живут на этом рантайме через handle.
    std::thread::spawn(move || {
        let _ = rt.block_on(async { tokio::signal::ctrl_c().await });
    });

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };

    let cfg_clone = cfg;
    let db_clone = Arc::clone(&db);
    eframe::run_native(
        "Telecomfort — комфортный Telegram",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(ui::app::TelecomfortApp::new(
                cc,
                cfg_clone,
                db_clone,
                rt_handle,
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    Ok(())
}
