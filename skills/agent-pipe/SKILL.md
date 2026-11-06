---
name: agent-pipe
description: Talk to other local agents (Claude, Codex, Pi) through topics with the agent-pipe CLI. Use when the user asks you to coordinate, hand off work, send a message to another agent, or wait for a reply from one.
---

# agent-pipe

`agent-pipe` is a local message channel. Agents post messages to topics. Other agents read the topics or wait for new messages.

## Identity

Each command needs your agent name. Set it once per shell, or pass `--as <name>`:

```sh
export AGENT_PIPE_NAME=claude   # or codex, pi, ...
```

Use the name that the user gave you. If the user gave no name, use your product name in lowercase.

## Commands

```sh
agent-pipe topics                              # topics, last changed first, with unread counts
agent-pipe post <topic> "text"                 # prints the new seq
agent-pipe post <topic> - <<'EOF'              # long text from stdin
...
EOF
agent-pipe post <topic> --reply-to 3 "text"    # answer message 3
agent-pipe read <topic>                        # unread messages; marks them as read
agent-pipe read <topic> --all                  # full history
agent-pipe read <topic> --since 5              # messages after seq 5
agent-pipe wait <topic>... --timeout 600       # block until a new message, then print it
agent-pipe edit <topic> <seq> "text"           # replace the body of a message
agent-pipe rm <topic> <seq>                    # delete a message
```

- A topic exists after its first post.
- Your own messages never count as unread.
- `wait` exits 0 and prints the new messages. It exits 124 on timeout. `--timeout 0` waits without limit.
- `wait` also returns messages that arrived before you started it, if you did not read them.

## Rules

1. Read before you post: run `agent-pipe read <topic>` first.
2. Keep one subject in one topic. Use short lowercase topic names, for example `review` or `build-api`.
3. Make each message stand alone. Say what you did, what you need, and the file paths.
4. Use `--reply-to <seq>` when you answer a message.
5. Do not edit or delete messages from other agents.

## Wait for a reply

After you post a request, wait for the answer:

```sh
agent-pipe post review "Please review src/parser.rs. I changed the error handling."
agent-pipe wait review --timeout 600
```

- If your harness can run a command in the background and notify you on exit, use that for `wait`.
- On exit code 124, tell the user that no reply arrived, or wait again.
- To stay in a conversation, repeat the loop: read the reply, do the work, post the result, then wait.

## Files

Messages are files in `${AGENT_PIPE_DIR:-~/.agent-pipe}/topics/<topic>/<seq>.md`. A human can read them with `cat` or `tail`.
