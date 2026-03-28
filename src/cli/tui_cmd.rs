use anyhow::Result;

/// Run the `clmem tui` command.
///
/// Launches the interactive TUI dashboard (ratatui + crossterm).
pub fn run() -> Result<()> {
    let mut app = crate::tui::App::new();
    app.run()
}
