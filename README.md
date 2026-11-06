# agent-pipe

A local pub/sub channel for agents such as Claude, Codex and Pi. It is one binary with no daemon. The messages are plain files that a human can read.

## Install

```sh
cargo install --path .
```

To teach the agents the commands, link the skill. Codex and Pi read `~/.agents/skills`. Claude Code reads `~/.claude/skills`.

```sh
ln -s "$PWD/skills/agent-pipe" ~/.agents/skills/agent-pipe
ln -s ../../.agents/skills/agent-pipe ~/.claude/skills/agent-pipe
```

## Use

```sh
export AGENT_PIPE_NAME=codex
agent-pipe post review "Please review src/parser.rs"     # prints 1

export AGENT_PIPE_NAME=claude
agent-pipe wait review --timeout 600                     # prints message 1
agent-pipe post review --reply-to 1 "Looks good"
agent-pipe topics
```

Run `agent-pipe --help` for all commands.

## Data layout

```
$AGENT_PIPE_DIR (default ~/.agent-pipe)
  topics/
    review/
      000001.md        # one message
      .cursors/claude  # last seq that claude read
      .trash/          # deleted messages
```

A message has header lines, a blank line, and a Markdown body:

```
from: codex
ts: 2026-11-06T10:14:03Z
reply-to: 1

Please review src/parser.rs
```

## Design

- A writer writes a temp file, then links it to the next free seq. The link fails if the name exists, so concurrent writers never get the same seq. A reader never sees a half-written file.
- `rm` moves a message to `.trash/`. Because of this, a seq is never used two times.
- `wait` checks the topic directory every 250 ms.
- Only new messages wake `wait`. `edit` does not notify.
