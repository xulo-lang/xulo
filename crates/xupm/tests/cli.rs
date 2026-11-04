use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_xupm");

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xupm_it_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project(dir: &Path, manifest: &str, entry: &str) {
    std::fs::write(dir.join("xulo.toml"), manifest).unwrap();
    std::fs::write(dir.join("main.xulo"), entry).unwrap();
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[test]
fn new_creates_a_manifest_and_entry() {
    let parent = temp_dir();
    let project = parent.join("demo");
    let out = run(&["new", project.to_str().unwrap()], &parent);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest = std::fs::read_to_string(project.join("xulo.toml")).unwrap();
    assert!(manifest.contains("name = \"demo\""));
    assert!(manifest.contains("entry = \"main.xulo\""));
    assert!(project.join("main.xulo").is_file());
}

#[test]
fn new_refuses_an_existing_destination() {
    let parent = temp_dir();
    let project = parent.join("demo");
    std::fs::create_dir_all(&project).unwrap();
    let out = run(&["new", project.to_str().unwrap()], &parent);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_uses_the_manifest_entry() {
    let dir = temp_dir();
    write_project(
        &dir,
        "[package]\nname = \"demo\"\nentry = \"main.xulo\"\n",
        "fn main() {\n    print(\"hello from the project\")\n}\n",
    );
    let out = run(&["run"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "hello from the project"
    );
}

#[test]
fn run_accepts_an_explicit_file() {
    let dir = temp_dir();
    write_project(
        &dir,
        "[package]\nname = \"demo\"\nentry = \"main.xulo\"\n",
        "fn main() {\n    print(\"from main\")\n}\n",
    );
    std::fs::write(
        dir.join("other.xulo"),
        "fn main() {\n    print(\"from other\")\n}\n",
    )
    .unwrap();
    let out = run(&["run", "other.xulo"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from other");
}

#[test]
fn run_outside_a_project_reports_the_missing_manifest() {
    let dir = temp_dir();
    std::fs::write(dir.join("main.xulo"), "fn main() {}\n").unwrap();
    let out = run(&["run"], &dir);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("could not find `xulo.toml`"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_runs_the_entry_modules_tests() {
    let dir = temp_dir();
    write_project(
        &dir,
        "[package]\nname = \"demo\"\nentry = \"main.xulo\"\n",
        "fn add(a: number, b: number): number { a + b }\n\n\
         #Test(\"addition\")\nfn addition() {\n    assert(add(2, 3) == 5, \"sum\")\n}\n\n\
         fn main() {}\n",
    );
    let out = run(&["test"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("test addition ... ok"), "stdout: {stdout}");
    assert!(stdout.contains("1 passed, 0 failed"), "stdout: {stdout}");
}
