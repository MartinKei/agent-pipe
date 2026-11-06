use assert_cmd::Command;
use std::{fs, path::Path, thread, time::Duration};

fn pipe(root: &Path, name: &str) -> Command {
    let mut cmd = Command::cargo_bin("agent-pipe").unwrap();
    cmd.env("AGENT_PIPE_DIR", root).env("AGENT_PIPE_NAME", name);
    cmd
}

fn out(cmd: &mut Command) -> String {
    String::from_utf8(cmd.assert().success().get_output().stdout.clone()).unwrap()
}

#[test]
fn post_read_and_cursor() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    assert_eq!(out(pipe(r, "codex").args(["post", "review", "hello"])), "1\n");
    assert_eq!(out(pipe(r, "codex").args(["post", "review", "--reply-to", "1", "-"]).write_stdin("again\n")), "2\n");

    let file = fs::read_to_string(r.join("topics/review/000002.md")).unwrap();
    assert!(file.starts_with("from: codex\nts: "));
    assert!(file.ends_with("reply-to: 1\n\nagain\n"));

    let first = out(pipe(r, "claude").args(["read", "review"]));
    assert!(first.contains("==> review/000001.md <==") && first.contains("again"));
    assert_eq!(out(pipe(r, "claude").args(["read", "review"])), "");
    assert_eq!(fs::read_to_string(r.join("topics/review/.cursors/claude")).unwrap(), "2\n");

    // own messages are never unread
    assert_eq!(out(pipe(r, "codex").args(["read", "review"])), "");
    assert!(out(pipe(r, "x").args(["read", "review", "--since", "1"])).contains("000002.md"));
    assert!(!out(pipe(r, "x").args(["read", "review", "--all"])).is_empty());
}

#[test]
fn wait_wakes_on_new_message_and_times_out() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path().to_path_buf();
    let r2 = r.clone();
    let poster = thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        out(pipe(&r2, "codex").args(["post", "build", "done"]));
    });
    let got = out(pipe(&r, "claude").args(["wait", "review", "build", "--timeout", "10"]));
    poster.join().unwrap();
    assert!(got.contains("==> build/000001.md <==") && got.contains("done"));

    pipe(&r, "claude").args(["wait", "build", "--timeout", "1"]).assert().code(124);
}

#[test]
fn topics_sorted_by_last_change() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    out(pipe(r, "a").args(["post", "old", "x"]));
    thread::sleep(Duration::from_millis(1100));
    out(pipe(r, "a").args(["post", "new", "x"]));
    let list = out(pipe(r, "b").arg("topics"));
    let new = list.find("new ").unwrap();
    let old = list.find("old ").unwrap();
    assert!(new < old, "{list}");

    thread::sleep(Duration::from_millis(1100));
    out(pipe(r, "a").args(["edit", "old", "1", "changed"]));
    let list = out(pipe(r, "b").arg("topics"));
    assert!(list.find("old ").unwrap() < list.find("new ").unwrap(), "{list}");
}

#[test]
fn edit_and_rm_without_seq_reuse() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    out(pipe(r, "a").args(["post", "t", "one"]));
    out(pipe(r, "a").args(["edit", "t", "1", "uno"]));
    let file = fs::read_to_string(r.join("topics/t/000001.md")).unwrap();
    assert!(file.contains("\nedited: ") && file.ends_with("\n\nuno\n"), "{file}");

    out(pipe(r, "a").args(["rm", "t", "1"]));
    assert!(r.join("topics/t/.trash/000001.md").exists());
    assert_eq!(out(pipe(r, "a").args(["post", "t", "two"])), "2\n");
    pipe(r, "a").args(["rm", "t", "1"]).assert().failure();
}

#[test]
fn concurrent_posts_get_unique_seqs() {
    let root = tempfile::tempdir().unwrap();
    let handles: Vec<_> = (0..20)
        .map(|i| {
            let r = root.path().to_path_buf();
            thread::spawn(move || out(pipe(&r, "a").args(["post", "t", &format!("m{i}")])))
        })
        .collect();
    let mut seqs: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap().trim().parse().unwrap()).collect();
    seqs.sort();
    assert_eq!(seqs, (1..=20).collect::<Vec<_>>());
}

#[test]
fn rejects_bad_names() {
    let root = tempfile::tempdir().unwrap();
    pipe(root.path(), "a").args(["post", "../x", "hi"]).assert().failure();
    pipe(root.path(), "../a").args(["post", "t", "hi"]).assert().failure();
}
