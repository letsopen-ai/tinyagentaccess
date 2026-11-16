use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use taa_tools::{
    config::Config, execute, screenshots::save_result, sequence_with_limits, SequenceLimits,
};
fn files(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .flat_map(|e| {
            let p = e.unwrap().path();
            if p.is_dir() {
                files(&p)
            } else {
                vec![p]
            }
        })
        .collect()
}
#[tokio::test]
async fn storage_layout_modes_and_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Arc::new(taa_platform::MockPlatform::recording());
    let mut c = Config::default();
    c.agent_access.screen_resolution = "native".into();
    for (format, ext) in [("png", "png"), ("jpeg", "jpg")] {
        c.agent_access.screen_image_format = format.into();
        let r = execute("screenshot", &json!({}), &c, p.clone()).await;
        assert_eq!(r["isError"], false);
        save_result("", "session-1", &r);
        assert!(files(tmp.path()).is_empty());
        save_result(tmp.path().to_str().unwrap(), "session-1", &r);
        let all = files(tmp.path());
        let file = all.iter().find(|p| p.extension().unwrap() == ext).unwrap();
        let relative = file
            .strip_prefix(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        assert!(relative.starts_with("agentaccess/screenshots/year="));
        assert!(relative.contains("/month="));
        assert!(relative.contains("/day="));
        assert!(relative.contains("/session-1/"));
        assert!(!file.file_name().unwrap().to_str().unwrap().contains(':'));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let mut parent = file.parent().unwrap();
            while parent != tmp.path() {
                assert_eq!(
                    fs::metadata(parent).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                parent = parent.parent().unwrap();
            }
        }
        let blocked = tmp.path().join("blocked");
        fs::write(&blocked, b"file").unwrap();
        save_result(blocked.to_str().unwrap(), "session-1", &r);
        assert_eq!(r["isError"], false);
        save_result(tmp.path().to_str().unwrap(), "../escape", &r);
        fs::remove_file(blocked).unwrap();
        for f in files(tmp.path()) {
            fs::remove_file(f).unwrap();
        }
    }
    let r = sequence_with_limits(
        &json!({"actions":[{"type":"wait","duration":0.001}]}),
        &c,
        p,
        SequenceLimits::default(),
    )
    .await;
    save_result(tmp.path().to_str().unwrap(), "terminal", &r);
    assert_eq!(files(tmp.path()).len(), 1);
    assert!(files(tmp.path())[0].to_str().unwrap().contains("terminal"));
}
#[cfg(unix)]
#[test]
fn refuses_directory_symlinks() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), tmp.path().join("agentaccess")).unwrap();
    save_result(
        tmp.path().to_str().unwrap(),
        "session",
        &json!({"content":[{"type":"image","data":"eA==","mimeType":"image/png"}]}),
    );
    assert!(files(outside.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn allows_symlink_ancestors_but_rejects_configured_symlink() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = tmp.path().join("alias");
    symlink(&real, &alias).unwrap();
    let image = json!({"content":[{"type":"image","data":"eA==","mimeType":"image/png"}]});
    for suffix in ["", "/"] {
        save_result(&format!("{}{suffix}", alias.display()), "session", &image);
        assert!(files(&real).is_empty());
    }
    fs::create_dir(real.join("configured")).unwrap();
    save_result(
        alias.join("configured").to_str().unwrap(),
        "session",
        &image,
    );
    assert_eq!(files(&real).len(), 1);
}
