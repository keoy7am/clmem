use anyhow::Result;

use crate::models::Config;
use crate::ConfigAction;

/// Run the `clmem config` command.
pub fn run(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => config_show(),
        ConfigAction::Edit => config_edit(),
        ConfigAction::Reset => config_reset(),
        ConfigAction::Path => config_path(),
    }
}

/// Display the current configuration as TOML.
fn config_show() -> Result<()> {
    let config = Config::load()?;
    let toml_str = toml::to_string_pretty(&config)?;
    println!("{}", toml_str);
    Ok(())
}

/// Open the config file in the user's editor.
fn config_edit() -> Result<()> {
    let path = Config::config_path()?;

    // Create the file with defaults if it doesn't exist
    if !path.exists() {
        tracing::info!("Config file does not exist, creating with defaults");
        Config::default().save()?;
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
        if cfg!(windows) {
            "notepad".to_string()
        } else {
            "vi".to_string()
        }
    });

    let parts: Vec<&str> = editor.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("EDITOR environment variable is empty");
    }

    println!("Opening {} with {}...", path.display(), editor);

    let status = std::process::Command::new(parts[0])
        .args(&parts[1..])
        .arg(&path)
        .status()?;

    if !status.success() {
        anyhow::bail!("Editor exited with status: {}", status);
    }

    Ok(())
}

/// Reset the configuration to defaults.
fn config_reset() -> Result<()> {
    let path = Config::config_path()?;
    Config::default().save()?;
    println!("Configuration reset to defaults at {}", path.display());
    Ok(())
}

/// Print the config file path.
fn config_path() -> Result<()> {
    let path = Config::config_path()?;
    println!("{}", path.display());
    Ok(())
}
