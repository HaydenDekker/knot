use std::fs;

#[test]
fn create_and_list_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    fs::write(dir.join("agent-1.json"), r#"{"name":"alpha"}"#).unwrap();
    fs::write(dir.join("agent-2.json"), r#"{"name":"beta"}"#).unwrap();

    let entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&"agent-1.json".to_string()));
}

#[test]
fn missing_file_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(fs::read_to_string(tmp.path().join("missing.json")).is_err());
}

#[test]
fn roundtrip_write_and_read() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.yaml");
    let content = "---\nagent:\n  mode: parallel\n";

    fs::write(&file, content).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), content);
}
