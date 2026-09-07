from pathlib import Path

path = Path("src-tauri/src/isolated_runtime.rs")
text = path.read_text(encoding="utf-8")

old = '''    #[test]
    fn sandbox_cwd_translation_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        assert_eq!(
            translate_sandbox_cwd(&workspace, "/workspace", "/workspace/src"),
            Some(display_path(&workspace.join("src")))
        );
        assert!(translate_sandbox_cwd(&workspace, "/workspace", "/other/src").is_none());
    }

'''
new = '''    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn sandbox_cwd_translation_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        assert_eq!(
            translate_sandbox_cwd(&workspace, "/workspace", "/workspace/src"),
            Some(display_path(&workspace.join("src")))
        );
        assert!(translate_sandbox_cwd(&workspace, "/workspace", "/other/src").is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_sandbox_cwd_translation_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        assert_eq!(
            translate_windows_sandbox_cwd(&workspace, r"C:\\OpenMindWorkspace\\src"),
            Some(display_path(&workspace.join("src")))
        );
        assert!(
            translate_windows_sandbox_cwd(&workspace, r"C:\\OtherWorkspace\\src").is_none()
        );
    }

'''

if text.count(old) != 1:
    raise SystemExit("expected Unix sandbox translation test exactly once")

path.write_text(text.replace(old, new, 1), encoding="utf-8")
