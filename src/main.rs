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
            .with_min_inner_size([960.0, 600.0])
            // Десктоп-интеграция: явный app id (путь персистентности eframe,
            // группировка окон, уведомления) + иконка дока/таскбара,
            // сгенерированная в коде (assets/ не требуем).
            .with_app_id("telecomfort")
            .with_icon(app_icon()),
        // Центрируем окно при старте (персистентность позиции не включаем —
        // eframe собран без фичи `persistence`, окно всегда новое).
        centered: true,
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

/// Иконка окна/дока: рисуем в коде, без assets (128x128 — кратно 4,
/// как требует [`egui::IconData`]). Тёмный скруглённый квадрат +
/// бирюзовая «линза» — отсылка к кадрам сторис и превью.
fn app_icon() -> egui::IconData {
    const S: u32 = 128;
    let mut img = image::RgbaImage::new(S, S);
    let c = S as f32 / 2.0;
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Скруглённый квадрат 26px, сглаживание 1px.
            let rad = 26.0f32;
            let qx = (fx - c).abs() - (c - rad);
            let qy = (fy - c).abs() - (c - rad);
            let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad;
            let alpha = (0.5 - dist).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            // Вертикальный градиент slate: #243247 → #0B1220.
            let t = fy / S as f32;
            let (br, bg, bb) = (
                36.0 + (11.0 - 36.0) * t,
                50.0 + (18.0 - 50.0) * t,
                71.0 + (32.0 - 71.0) * t,
            );
            let mut px = [br as u8, bg as u8, bb as u8, (alpha * 255.0) as u8];
            // «Линза»: бирюзовый круг r=34 с тёмной сердцевиной и бликом.
            let dl = (fx - c).hypot(fy - c);
            if dl < 34.0 {
                let lt = (dl / 34.0).clamp(0.0, 1.0);
                // Центр #5EEAD4 → край #0E7490.
                let (lr, lg, lb) = (
                    94.0 + (14.0 - 94.0) * lt,
                    234.0 + (116.0 - 234.0) * lt,
                    212.0 + (144.0 - 212.0) * lt,
                );
                px[0] = lr as u8;
                px[1] = lg as u8;
                px[2] = lb as u8;
                if dl < 15.0 {
                    // Сердцевина.
                    px = [11, 18, 32, (alpha * 255.0) as u8];
                    // Блик вверху слева сердцевины.
                    let dg = (fx - (c - 6.0)).hypot(fy - (c - 6.0));
                    if dg < 5.0 {
                        let k = 1.0 - dg / 5.0;
                        px[0] = (px[0] as f32 + (255.0 - px[0] as f32) * k) as u8;
                        px[1] = (px[1] as f32 + (255.0 - px[1] as f32) * k) as u8;
                        px[2] = (px[2] as f32 + (255.0 - px[2] as f32) * k) as u8;
                    }
                }
            }
            img.put_pixel(x, y, image::Rgba(px));
        }
    }
    egui::IconData { rgba: img.into_raw(), width: S, height: S }
}
