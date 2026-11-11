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
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("xupm.toml"), manifest).unwrap();
    std::fs::write(dir.join("main.xulo"), entry).unwrap();
}

fn write_workspace(dir: &Path, member_names: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("xupm.toml"),
        "[workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    for name in member_names {
        write_project(
            &dir.join("packages").join(name),
            &format!("[package]\nname = \"{name}\"\nentry = \"main.xulo\"\n"),
            &format!("fn main() {{\n    print(\"{name}\")\n}}\n"),
        );
    }
}

fn write_package_test(root: &Path, name: &str, test_name: &str) {
    write_project(
        &root.join("packages").join(name),
        &format!("[package]\nname = \"{name}\"\nentry = \"main.xulo\"\n"),
        &format!(
            "#[test]\nfn {test_name}() {{\n    assert(1 + 1 == 2, \"math\")\n}}\n\nfn main() {{}}\n"
        ),
    );
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
    let manifest = std::fs::read_to_string(project.join("xupm.toml")).unwrap();
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
        String::from_utf8_lossy(&out.stderr).contains("could not find `xupm.toml`"),
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
         #[test]\nfn addition() {\n    assert(add(2, 3) == 5, \"sum\")\n}\n\n\
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

#[test]
fn run_in_a_virtual_workspace_requires_a_package() {
    let dir = temp_dir();
    write_workspace(&dir, &["alpha", "beta"]);
    let out = run(&["run"], &dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("select one with `-p/--package"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("alpha"), "stderr: {stderr}");
    assert!(stderr.contains("beta"), "stderr: {stderr}");

    let out = run(&["run", "-p", "beta"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "beta");
}

#[test]
fn test_in_a_virtual_workspace_runs_every_member() {
    let dir = temp_dir();
    write_workspace(&dir, &["alpha", "beta"]);
    write_package_test(&dir, "alpha", "alpha_test");
    write_package_test(&dir, "beta", "beta_test");
    let out = run(&["test"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("test alpha_test ... ok"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("test beta_test ... ok"), "stdout: {stdout}");
}

#[test]
fn package_flag_selects_one_workspace_member() {
    let dir = temp_dir();
    write_workspace(&dir, &["alpha", "beta"]);
    write_package_test(&dir, "alpha", "alpha_test");
    write_package_test(&dir, "beta", "beta_test");
    let out = run(&["test", "-p", "alpha"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("test alpha_test ... ok"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("test beta_test"), "stdout: {stdout}");
}

#[test]
fn unknown_package_lists_available_members() {
    let dir = temp_dir();
    write_workspace(&dir, &["alpha", "beta"]);
    let out = run(&["run", "-p", "missing"], &dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no package named `missing`"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("alpha, beta"), "stderr: {stderr}");
}

#[test]
fn root_package_defaults_to_itself_and_workspace_includes_members() {
    let dir = temp_dir();
    std::fs::write(
        dir.join("xupm.toml"),
        "[package]\nname = \"root\"\nentry = \"main.xulo\"\n\n\
         [workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.xulo"),
        "fn main() {\n    print(\"root\")\n}\n",
    )
    .unwrap();
    write_project(
        &dir.join("packages").join("member"),
        "[package]\nname = \"member\"\nentry = \"main.xulo\"\n",
        "fn main() {\n    print(\"member\")\n}\n",
    );

    let out = run(&["run"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "root");

    let out = run(&["check"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("no errors").count(), 1, "stdout: {stdout}");

    let out = run(&["check", "--workspace"], &dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("no errors").count(), 2, "stdout: {stdout}");
}

#[test]
fn explicit_file_cannot_be_combined_with_a_package() {
    let dir = temp_dir();
    write_workspace(&dir, &["alpha"]);
    let out = run(&["run", "main.xulo", "-p", "alpha"], &dir);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be combined"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
