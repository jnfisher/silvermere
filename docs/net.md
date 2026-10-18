# silvermere-net: the telnet transport

The network edge. This page records what the crate
does and where it differs from the original host (Worldgroup GSBL) or from MBBSEmu's
`TelnetSession`.

## Pieces

| Module | What |
|---|---|
| `telnet::parser` | `Parser::feed(byte) -> Option<Action>`: the IAC state machine, pure. |
| `telnet::options` | The offers sent on accept and the answer table. |
| `telnet` | `TelnetCodec` (parser + options + NAWS, no I/O), `escape_into` (0xFF doubling), `TelnetTransport` (sequential `Transport` over a `TcpStream`). |
| `line_editor` | `LineEditor`: the GSBL line discipline and echo. |
| `session_task` | `spawn_session`, `serve`, `bind`: one task per connection, the accept loop. |
| `events` | `SessionEvent`, `SessionCommand`, `Reason`, `Capabilities`, `SessionId`. |
| `cp437` | Input decode table (until T-010's codec replaces it). |

## Negotiation

On accept the server sends exactly `IAC WILL ECHO`, `IAC WILL SGA`, `IAC WILL BINARY`,
`IAC DO NAWS`, in that order, and emits `SessionEvent::Connected(80x24)` at once: the login
prompt never waits for answers.

| Client sends | Server answers |
|---|---|
| `DO ECHO` / `DO SGA` / `DO BINARY` | `WILL` (once; repeated `DO`s are not re-answered) |
| `DO` anything else (`TTYPE`, ...) | `WONT` |
| `WILL NAWS` | `DO NAWS` (once) |
| `WILL` anything else (`LINEMODE`, ...) | `DONT` |
| `WONT` / `DONT` anything | nothing; the refusal is accepted and never retried |
| `SB NAWS w h SE` | `SessionEvent::Naws(w, h)`; a zero dimension keeps the default |

Differences from MBBSEmu: it also sends `DO BINARY`, `DONT ECHO`, `DO SGA` and `WONT NAWS`
on accept. Those are dropped; MegaMud ignores them, and a client that answers
every `DO` with `WONT` and ignores `WILL` works either way. The server echoes whatever the
client answered about `ECHO`, as the original host did. `TTYPE` is refused.
No keepalive by default; `SessionConfig::heartbeat`
sends `IAC NOP` after that long without output (MBBSEmu's `Telnet.Heartbeat`, off in our
config). MBBSEmu's 5-minute receive timeout is not kept: idle hang-up is the engine's rule
(`INACTIVE`, T-037), which sees the time of each `Line`/`Key` event.

## Line discipline

- `CR LF`, `CR NUL`, bare `CR` and bare `LF` each end a line; the terminator echoes `\r\n`.
- `BS` and `DEL` erase one byte and echo `BS SP BS`.
- Other control bytes, XOFF (0x13) included, are dropped.
- Bytes with the high bit set are **kept** (the GSBL default translation table stripped the
  high bit; names are ASCII, but CP437 input is harmless and lossless this way). `IAC IAC`
  arrives as a data 0xFF and is kept.
- Limit 254 bytes everywhere (MBBSEmu's `btumil` set to 0xFE at realm entry; the BBS
  menus used the host default 255, and one limit is fine). The 255th byte discards the line
  and emits `SessionEvent::LineTooLong` (GSBL status 251); the engine picks the message.
- Echo happens when the byte arrives, so type-ahead splices into output as on the original
  (see the manual check below: `Kasimir` is echoed before the `Name:` prompt it answers).
- `EchoMode::Secure { ch: b'*', width: 8 }` (MBBSEmu's `echsec` with `*` and 8) echoes one `*`
  per byte up to 8 and nothing beyond, and lasts one line. `EchoMode::Off` echoes nothing,
  not even the terminator.
- `SetCharMode(true)` delivers `SessionEvent::Key(byte)` per keystroke with no editing or
  echo. `CR LF`/`CR NUL` still fold to one `CR` key and
  bare `LF` becomes `CR`; ESC sequences are the engine's.

## Queues and backpressure

- Input: completed lines wait in the editor (cap 64; the original held two). While 64 are
  waiting the session stops reading its socket, so TCP holds the rest: nothing is dropped
  however slow the engine is. Lines are delivered one event per `recv` on the engine side,
  in order.
- Output: `SessionCommand::Write` bytes are queued (after 0xFF doubling) and written FIFO.
  If more than 64 KiB is waiting the session is dropped with a log line and
  `Disconnect(Reason::Backpressure)`. The listener's `SO_SNDBUF` is capped at 32 KiB by
  `bind` (accepted sockets inherit it); otherwise the kernel would buffer megabytes for a
  stalled client before the queue bound noticed.
- The engine should `try_send` commands so it never waits on a session.
- `Disconnect(Reason::Eof)` for a client close or reset (lines typed before the close are
  delivered first), `Closed` after `SessionCommand::Close` (queued output is flushed first,
  up to 5 s), `Io(kind)` for anything else.

## Accept loop

`serve(listener, config, to_engine, registry)` assigns `SessionId`s, sends each session's
command sender on `registry` before starting its task (so when the engine sees a
`Connected` for an unknown id, the registration is already waiting in `registry`), and over
`max_sessions` writes `compat.full_message` (`The server is full. Please try again later.`)
and closes. One connection per account is the engine's rule (it knows accounts).

## Manual check

```
cargo run -p silvermere-net --example telnet_echo        # listens on 127.0.0.1:2323
nc localhost 2323 | od -c                                 # in another terminal
```

The first twelve bytes are the four offers, then typing is echoed. A scripted run
(`Kasimir`, then `pw` to switch to secure echo, then `secret`) shows:

```
0000000 377 373 001 377 373 003 377 373  \0 377 375 037   K   a   s   i
0000020   m   i   r  \r  \n   N   a   m   e   :       Y   o   u       t
0000040   y   p   e   d   :       K   a   s   i   m   i   r  \r  \n   N
0000060   a   m   e   :       p   w  \r  \n   P   a   s   s   w   o   r
0000100   d   :       *   *   *   *   *   *  \r  \n   Y   o   u       t
0000120   y   p   e   d   :       s   e   c   r   e   t  \r  \n   N   a
```

`377 373 001` is `IAC WILL ECHO`, then `WILL SGA` (003), `WILL BINARY` (\0) and `IAC DO NAWS`
(`377 375 037`). Interactively, `telnet localhost 2323` shows the same prompt with the
client's own echo turned off.
