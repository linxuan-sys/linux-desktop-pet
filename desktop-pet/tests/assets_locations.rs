use std::{
    path::Path,
    process::{Command, Output},
};
fn command(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pet"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("PET_ASSET_BASE_URL", "http://127.0.0.1:9")
        .output()
        .unwrap()
}
fn okay(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn media(path: &Path) {
    std::fs::create_dir_all(path.join("anim")).unwrap();
    std::fs::write(path.join("anim/idle.webp"), b"test original bytes").unwrap();
}
#[test]
fn source_checkout_is_remembered_without_embedding_build_paths() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let repo = dir.path().join("fork");
    let source = repo.join("claude-girl/media");
    media(&source);
    std::fs::create_dir_all(&home).unwrap();
    let first = okay(command(
        &home,
        &repo,
        &["run", "claude", "--download-assets"],
    ));
    assert!(first.contains(source.to_str().unwrap()));
    let second = okay(command(
        &home,
        &home,
        &["run", "claude", "--download-assets"],
    ));
    assert!(second.contains(source.to_str().unwrap()));
    assert!(!home.join("data/claude-girl").exists());
}
#[test]
fn chosen_library_and_single_media_take_priority_over_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let repo = dir.path().join("repo");
    media(&repo.join("claude-girl/media"));
    let library = dir.path().join("library");
    let source = library.join("claude-girl/media");
    media(&source);
    okay(command(
        &home,
        &repo,
        &["assets-dir", library.to_str().unwrap()],
    ));
    let output = okay(command(
        &home,
        &repo,
        &["run", "claude", "--download-assets"],
    ));
    assert!(output.contains(source.to_str().unwrap()));
    let direct = dir.path().join("custom-media");
    media(&direct);
    okay(command(
        &home,
        &repo,
        &["assets-dir", "claude", direct.to_str().unwrap()],
    ));
    let output = okay(command(
        &home,
        &repo,
        &["run", "claude", "--download-assets"],
    ));
    assert!(output.contains(direct.to_str().unwrap()));
    let invalid = command(
        &home,
        &repo,
        &["assets-dir", "claude", home.to_str().unwrap()],
    );
    assert!(!invalid.status.success());
    let output = okay(command(
        &home,
        &repo,
        &["run", "claude", "--download-assets"],
    ));
    assert!(
        output.contains(direct.to_str().unwrap()),
        "invalid input must preserve previous source"
    );
    okay(command(&home, &repo, &["assets-dir", "default"]));
    let output = okay(command(
        &home,
        &repo,
        &["run", "claude", "--download-assets"],
    ));
    assert!(output.contains(repo.join("claude-girl/media").to_str().unwrap()));
}
