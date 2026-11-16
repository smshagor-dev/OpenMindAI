//! Conservative recognition of one direct validation invocation. This is host
//! execution evidence, not proof that project-owned scripts contain real tests.
pub(super) fn successful(command: &str, exit_code: i32, timed_out: bool) -> Option<String> {
    (exit_code == 0 && !timed_out && recognized(command)).then(|| command.to_string())
}

pub(super) fn recognized(command: &str) -> bool {
    // Accept only unquoted simple argv. Shell syntax, expansion, comments,
    // redirects, executable paths and wrappers need independently attested runs.
    if command.is_empty()
        || command.chars().any(|c| {
            !(c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '\t' | '-' | '_' | '.' | '/' | ':' | '=' | ','))
        })
    {
        return false;
    }
    let args: Vec<_> = command.split_ascii_whitespace().collect();
    if args.is_empty() {
        return false;
    }
    for arg in &args {
        let lower = arg.to_ascii_lowercase();
        if [
            "--help",
            "--version",
            "--list",
            "--collect-only",
            "--no-run",
            "--dry-run",
            "--if-present",
            "--ignore-scripts",
            "--exclude-task",
            "--fix",
            "--write",
            "--setup-plan",
            "--fixtures",
            "--markers",
            "--print-config",
            "--showconfig",
            "--show-settings",
            "--show-files",
            "--build-plan",
            "--unit-graph",
            "--listfilesonly",
        ]
        .iter()
        .any(|flag| lower.contains(flag))
            || matches!(
                *arg,
                "-h" | "-V" | "-VV" | "-x" | "--co" | "-list" | "-version" | "-help"
            )
            || lower.contains("failure.ignore")
            || lower.contains("ignorefailures")
            || lower.contains("listtests")
            || (lower.contains("skip") && lower.contains("test"))
        {
            return false;
        }
    }
    let rest = &args[1..];
    match args[0] {
        "cargo" => {
            matches!(
                rest.first(),
                Some(&"test" | &"check" | &"clippy" | &"build")
            ) || (rest.first() == Some(&"fmt") && rest.contains(&"--check"))
        }
        "npm" => {
            !rest.contains(&"-v")
                && (rest.first() == Some(&"test")
                    || (rest.first() == Some(&"run")
                        && rest.get(1).is_some_and(|name| {
                            matches!(*name, "test" | "lint" | "build" | "typecheck")
                        })))
        }
        "pnpm" | "yarn" => matches!(rest.first(), Some(&"test" | &"lint" | &"build")),
        "pytest" | "phpunit" | "eslint" => true,
        "python" | "python3" => rest.starts_with(&["-m", "pytest"]),
        "go" => matches!(rest.first(), Some(&"test" | &"vet")),
        "mvn" => matches!(rest.first(), Some(&"test" | &"verify")),
        "gradle" | "dotnet" | "composer" => rest.first() == Some(&"test"),
        "tsc" => rest.contains(&"--noEmit"),
        "ruff" => rest.first() == Some(&"check"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_direct_runner_and_subcommand_argv() {
        for command in [
            "cargo test",
            "cargo clippy --all-targets",
            "cargo fmt -- --check",
            "npm run lint",
            "npm run build",
            "npm test",
            "pnpm test",
            "yarn lint",
            "python -m pytest -q",
            "python3 -m pytest tests/unit",
            "go vet ./...",
            "mvn verify",
            "gradle test",
            "dotnet test",
            "composer test",
            "phpunit",
            "eslint src",
            "tsc --noEmit",
            "ruff check .",
        ] {
            assert!(recognized(command), "{command}");
        }
    }
    #[test]
    fn rejects_echo_comments_wrappers_and_executable_lookalikes() {
        for command in [
            "echo cargo test",
            "printf npm run lint",
            "Write-Output cargo test",
            "true # cargo test",
            "env cargo test",
            "command cargo test",
            "bash -c cargo test",
            "cmd /c npm test",
            "npx eslint .",
            "./cargo test",
            "cargo-test",
            "npm run linted",
            "python -c pytest",
            "npm install",
            "gradlew test",
            "CARGO test",
        ] {
            assert!(!recognized(command), "{command}");
        }
    }
    #[test]
    fn rejects_shell_syntax_even_when_a_runner_name_occurs() {
        for command in [
            "cargo test && true",
            "cargo test || true",
            "npm test; exit 0",
            "cargo test | cat",
            "cargo test &",
            "cargo test > result",
            "cargo test < input",
            "cargo test\necho done",
            "cargo test\r",
            "cargo test # ignored",
            "cargo test $(echo foo)",
            "cargo test `echo foo`",
            "cargo test %FLAGS%",
            "cargo test -- --test-threads=$N",
            "'cargo' test",
            "cargo test \"filter\"",
            "cargo test *",
        ] {
            assert!(!recognized(command), "{command}");
        }
    }
    #[test]
    fn help_collection_skipping_and_mutation_do_not_count_as_validation() {
        for command in [
            "cargo test --help",
            "cargo test --no-run",
            "cargo test -- --list",
            "pytest --collect-only",
            "pytest --co",
            "npm run test --if-present",
            "npm test --dry-run",
            "npm test --ignore-scripts",
            "npm test -v",
            "mvn test -DskipTests",
            "mvn test -Dmaven.test.skip=true",
            "gradle test -x test",
            "dotnet test --list-tests",
            "eslint . --fix",
            "ruff check --fix",
            "tsc --version",
        ] {
            assert!(!recognized(command), "{command}");
        }
    }
    #[test]
    fn only_a_successful_non_timeout_host_result_attests_the_command() {
        assert_eq!(
            successful("cargo test", 0, false).as_deref(),
            Some("cargo test")
        );
        assert!(successful("cargo test", 1, false).is_none());
        assert!(successful("cargo test", 0, true).is_none());
        assert!(successful("echo cargo test", 0, false).is_none());
        assert!(successful("", 0, false).is_none());
    }
    #[test]
    fn echoing_a_test_command_does_not_satisfy_the_host_gate() {
        #[cfg(unix)]
        let result = std::process::Command::new("sh")
            .args(["-c", "echo cargo test"])
            .output()
            .unwrap();
        #[cfg(windows)]
        let result = std::process::Command::new("cmd")
            .args(["/C", "echo cargo test"])
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("cargo test"));
        assert!(successful("echo cargo test", result.status.code().unwrap(), false).is_none());
    }
}
