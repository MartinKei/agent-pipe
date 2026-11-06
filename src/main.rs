//! agent-pipe: a local pub/sub channel for agents.
//!
//! Layout: $AGENT_PIPE_DIR/topics/<topic>/<seq:06>.md, one file per message.
//! Each agent has a cursor file `.cursors/<name>` per topic with the last seq it read.
//! Deleted messages move to `.trash/`, so a seq is never used two times.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use jiff::Timestamp;
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// Local pub/sub channel for agents. Data is in $AGENT_PIPE_DIR (default ~/.agent-pipe).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Agent name. Required for post, wait and unread reads.
    #[arg(long = "as", global = true, env = "AGENT_PIPE_NAME")]
    name: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List topics, last changed first
    Topics,
    /// Post a message and print its seq. TEXT omitted or "-" reads stdin
    Post {
        topic: String,
        text: Option<String>,
        #[arg(long)]
        reply_to: Option<u64>,
    },
    /// Print unread messages and advance the cursor
    Read {
        topic: String,
        /// Print all messages
        #[arg(long)]
        all: bool,
        /// Print messages with a seq greater than N
        #[arg(long, conflicts_with = "all")]
        since: Option<u64>,
    },
    /// Block until a topic has unread messages, then print them. Exit 124 on timeout
    Wait {
        #[arg(required = true)]
        topics: Vec<String>,
        /// Seconds; 0 waits without limit
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Replace the body of a message. TEXT omitted or "-" reads stdin
    Edit { topic: String, seq: u64, text: Option<String> },
    /// Delete a message (moves it to the topic's .trash/)
    Rm { topic: String, seq: u64 },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let me = || -> Result<&str> {
        let n = cli.name.as_deref().context("set AGENT_PIPE_NAME or pass --as <name>")?;
        valid(n)
    };
    match &cli.cmd {
        Cmd::Topics => topics(cli.name.as_deref()),
        Cmd::Post { topic, text, reply_to } => {
            let dir = topic_dir(topic)?;
            fs::create_dir_all(&dir)?;
            let reply = reply_to.map(|r| format!("reply-to: {r}\n")).unwrap_or_default();
            let content = format!("from: {}\nts: {}\n{reply}\n{}", me()?, now(), body(text)?);
            loop {
                let next = seqs(&dir)?.into_iter().chain(seqs(&dir.join(".trash"))?).max().unwrap_or(0) + 1;
                if write_file(&dir, &msg_path(&dir, next), &content, true)? {
                    println!("{next}");
                    return Ok(());
                }
            }
        }
        Cmd::Read { topic, all, since } => {
            let dir = existing_topic(topic)?;
            let list = seqs(&dir)?;
            let shown = match (all, since) {
                (true, _) => list.clone(),
                (_, Some(n)) => list.iter().copied().filter(|s| s > n).collect(),
                _ => unread(&dir, &list, me()?)?,
            };
            print_msgs(topic, &dir, &shown)?;
            if let (Some(name), Some(&last)) = (cli.name.as_deref(), list.last()) {
                set_cursor(&dir, valid(name)?, last)?;
            }
            Ok(())
        }
        Cmd::Wait { topics, timeout } => {
            let name = me()?;
            let start = Instant::now();
            loop {
                let mut found = false;
                for topic in topics {
                    let dir = topic_dir(topic)?;
                    let list = seqs(&dir)?;
                    let new = unread(&dir, &list, name)?;
                    if !new.is_empty() {
                        print_msgs(topic, &dir, &new)?;
                        set_cursor(&dir, name, *list.last().unwrap())?;
                        found = true;
                    }
                }
                if found {
                    return Ok(());
                }
                if *timeout > 0 && start.elapsed() >= Duration::from_secs(*timeout) {
                    eprintln!("timeout");
                    std::process::exit(124);
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
        Cmd::Edit { topic, seq, text } => {
            let dir = existing_topic(topic)?;
            let path = msg_path(&dir, *seq);
            let old = fs::read_to_string(&path).with_context(|| format!("no message {topic}/{seq}"))?;
            let head: String = old.split("\n\n").next().unwrap_or("").lines()
                .filter(|l| !l.starts_with("edited: "))
                .map(|l| format!("{l}\n"))
                .collect();
            write_file(&dir, &path, &format!("{head}edited: {}\n\n{}", now(), body(text)?), false)?;
            Ok(())
        }
        Cmd::Rm { topic, seq } => {
            let dir = existing_topic(topic)?;
            let trash = dir.join(".trash");
            fs::create_dir_all(&trash)?;
            let name = format!("{seq:06}.md");
            fs::rename(dir.join(&name), trash.join(&name)).with_context(|| format!("no message {topic}/{seq}"))
        }
    }
}

fn topics(name: Option<&str>) -> Result<()> {
    let root = root()?.join("topics");
    let mut rows = Vec::new();
    for entry in fs::read_dir(&root).into_iter().flatten() {
        let entry = entry?;
        let topic = entry.file_name().to_string_lossy().into_owned();
        if topic.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let list = seqs(&dir)?;
        let mut changed = mtime(&dir)?;
        for s in &list {
            changed = changed.max(mtime(&msg_path(&dir, *s))?);
        }
        let unread = match name {
            Some(n) => unread(&dir, &list, valid(n)?)?.len().to_string(),
            None => "-".into(),
        };
        rows.push((changed, topic, list.len(), unread));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let w = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).max(5);
    println!("{:w$}  {:>5}  {:>6}  CHANGED", "TOPIC", "MSGS", "UNREAD");
    for (changed, topic, count, unread) in rows {
        let ts = Timestamp::try_from(changed)?;
        println!("{topic:w$}  {count:>5}  {unread:>6}  {}", Timestamp::from_second(ts.as_second())?);
    }
    Ok(())
}

fn root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("AGENT_PIPE_DIR") {
        return Ok(dir.into());
    }
    Ok(std::env::home_dir().context("no home directory")?.join(".agent-pipe"))
}

/// Accept names that are safe as one path component: [A-Za-z0-9._-], not starting with '.'.
fn valid(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        bail!("invalid name {name:?}: use letters, digits, '.', '_', '-'");
    }
    Ok(name)
}

fn topic_dir(topic: &str) -> Result<PathBuf> {
    Ok(root()?.join("topics").join(valid(topic)?))
}

fn existing_topic(topic: &str) -> Result<PathBuf> {
    let dir = topic_dir(topic)?;
    if !dir.is_dir() {
        bail!("no topic {topic}");
    }
    Ok(dir)
}

fn msg_path(dir: &Path, seq: u64) -> PathBuf {
    dir.join(format!("{seq:06}.md"))
}

/// Sorted message seqs in `dir`. A missing directory has no messages.
fn seqs(dir: &Path) -> Result<Vec<u64>> {
    let mut list: Vec<u64> = match fs::read_dir(dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(vec![]),
        r => r?
            .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".md")?.parse().ok())
            .collect(),
    };
    list.sort_unstable();
    Ok(list)
}

/// Seqs after the cursor of `name`, without the messages that `name` posted.
fn unread(dir: &Path, list: &[u64], name: &str) -> Result<Vec<u64>> {
    let cursor = cursor(dir, name);
    let mut out = Vec::new();
    for &s in list.iter().filter(|&&s| s > cursor) {
        let content = fs::read_to_string(msg_path(dir, s)).unwrap_or_default(); // rm can race
        if header(&content, "from") != Some(name) {
            out.push(s);
        }
    }
    Ok(out)
}

fn header<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    content.lines().take_while(|l| !l.is_empty()).find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
}

fn cursor(dir: &Path, name: &str) -> u64 {
    fs::read_to_string(dir.join(".cursors").join(name)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// Move the cursor forward. It never moves back.
fn set_cursor(dir: &Path, name: &str, seq: u64) -> Result<()> {
    if seq > cursor(dir, name) {
        let cursors = dir.join(".cursors");
        fs::create_dir_all(&cursors)?;
        write_file(&cursors, &cursors.join(name), &format!("{seq}\n"), false)?;
    }
    Ok(())
}

/// Write atomically through a temp file. With `noclobber`, return false if `path` exists.
fn write_file(dir: &Path, path: &Path, content: &str, noclobber: bool) -> Result<bool> {
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    if !noclobber {
        tmp.persist(path)?;
        return Ok(true);
    }
    match tmp.persist_noclobber(path) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.error.into()),
    }
}

fn print_msgs(topic: &str, dir: &Path, list: &[u64]) -> Result<()> {
    for &s in list {
        let Ok(content) = fs::read_to_string(msg_path(dir, s)) else { continue }; // rm can race
        println!("==> {topic}/{s:06}.md <==\n{content}");
    }
    Ok(())
}

fn body(text: &Option<String>) -> Result<String> {
    let mut s = match text.as_deref() {
        Some(t) if t != "-" => t.to_string(),
        _ => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
    };
    s.truncate(s.trim_end().len());
    if s.is_empty() {
        bail!("empty message");
    }
    Ok(s + "\n")
}

fn now() -> Timestamp {
    Timestamp::from_second(Timestamp::now().as_second()).unwrap()
}

fn mtime(path: &Path) -> Result<SystemTime> {
    Ok(fs::metadata(path)?.modified()?)
}
