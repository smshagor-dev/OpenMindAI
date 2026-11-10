use crate::app_error::AppError;
use std::{
    path::Path,
    process::{Command, Stdio},
};

fn command_for(platform: &str, path: &Path) -> Result<Command, AppError> {
    let executable = match platform {
        "windows" => "explorer.exe",
        "macos" => "open",
        "linux" => "xdg-open",
        _ => {
            return Err(AppError::internal(
                "Opening folders is unsupported on this platform",
            ))
        }
    };
    let mut command = Command::new(executable);
    if platform == "macos" {
        command.arg("--");
    }
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

fn launch(mut command: Command) -> Result<(), AppError> {
    let mut child = command.spawn().map_err(|error| {
        AppError::internal(format!("Could not launch the system file manager: {error}"))
    })?;
    // Reap short-lived launchers without blocking the desktop UI.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn open_folder(path: &Path) -> Result<(), AppError> {
    // Resolve to an absolute path so a folder starting with '-' is never an
    // option to xdg-open. Pass it as one argument, without shell interpretation.
    let path = std::fs::canonicalize(path)?;
    if !path.is_dir() {
        return Err(AppError::internal(
            "The requested folder is not a directory",
        ));
    }
    launch(command_for(std::env::consts::OS, &path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_desktop_platform_uses_its_file_manager_with_one_path_argument() {
        let path = Path::new("/test/বাংলা folder/quote' $value");
        for (platform, program) in [
            ("windows", "explorer.exe"),
            ("macos", "open"),
            ("linux", "xdg-open"),
        ] {
            let command = command_for(platform, path).unwrap();
            assert_eq!(command.get_program(), program);
            let args = command.get_args().collect::<Vec<_>>();
            assert_eq!(args.last().unwrap(), &path.as_os_str());
            assert_eq!(args.len(), if platform == "macos" { 2 } else { 1 });
        }
        assert!(command_for("unknown", path).is_err());
    }
    #[test]
    fn missing_or_regular_file_is_rejected_before_launch() {
        let temp = tempfile::tempdir().unwrap();
        assert!(open_folder(&temp.path().join("missing")).is_err());
        let file = temp.path().join("file");
        std::fs::write(&file, b"fixture").unwrap();
        assert!(open_folder(&file).is_err());
    }
    #[test]
    fn unavailable_launcher_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let command = Command::new(temp.path().join("missing-launcher"));
        assert!(launch(command)
            .unwrap_err()
            .to_string()
            .contains("Could not launch the system file manager"));
    }
}
