use anyhow::Result;

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Starting RiffLab");

    // TODO: parse CLI args, init library, spawn workers, create audio engine
    // For now, just launch the UI
    rifflab_ui::app::run().map_err(|e| anyhow::anyhow!("UI error: {e}"))?;

    Ok(())
}
