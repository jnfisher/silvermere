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
| `session_task` | `spawn_session`, `serve`, `serve_with_rlogin`, `bind`: one task per connection, the accept loops. |
| `rlogin` | The rlogin door login: `RloginConfig` (the trusted addresses), `RloginHello`, the handshake parser. |
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
| `DO GMCP` / `DONT GMCP` | nothing (it answers our offer); `SessionEvent::Gmcp(true/false)`. Without `net.gmcp`, `DO GMCP` gets `WONT` |
| `SB GMCP <message> SE` | `SessionEvent::GmcpMessage(bytes)`, once GMCP is agreed |

Differences from MBBSEmu: it also sends `DO BINARY`, `DONT ECHO`, `DO SGA` and `WONT NAWS`
on accept. Those are dropped; MegaMud ignores them, and a client that answers
every `DO` with `WONT` and ignores `WILL` works either way. The server echoes whatever the
client answered about `ECHO`, as the original host did. `TTYPE` is refused.
No keepalive by default; `SessionConfig::heartbeat`
sends `IAC NOP` after that long without output (MBBSEmu's `Telnet.Heartbeat`, off in our
config). MBBSEmu's 5-minute receive timeout is not kept: idle hang-up is the engine's rule
(`INACTIVE`, T-037), which sees the time of each `Line`/`Key` event.

## GMCP

With `SessionConfig::gmcp` (`net.gmcp`, on by default) the offers end with `IAC WILL GMCP`
(option 201, <https://tintin.mudhalla.net/protocols/gmcp/>). Once the client answers
`DO GMCP`, `SessionCommand::Gmcp(message)` goes out as `IAC SB GMCP <message> IAC SE` with
0xFF doubled; before that, or after `DONT GMCP`, or on rlogin, the command is dropped. GMCP
is neither recorded for the sysop watching a player nor CP437: the message is UTF-8
`Package.Name json`. Subnegotiations longer than 8 KiB are dropped whole. While 64 NAWS/GMCP
events wait for the engine the session stops reading, as for lines.

What the engine sends is in `silvermere-engine`'s `gmcp` module:

| Package | When | Data |
|---|---|---|
| `Room.Info` | each room display (moving, `LOOK`) | `num` (`map * 1000000 + room`), `name`, `area` (`Map <n>`), `environment` (room type), `map`, `room`, `exits` (`{"n": num, ...}`) |
| `Char.Name` | entering the realm | `name`, `fullname` |
| `Char.Status` | entering the realm, and on change | `name`, `fullname`, `level`, `class`, `race`, `exp` |
| `Char.Vitals` | entering the realm, and on change | `hp`, `maxhp`, `mp`, `maxmp` |
| `Core.Ping` | answering the client's `Core.Ping` | none |

`Room.Info` comes with the room display, so a player who cannot see gets none, and its
exits are the `Obvious exits:` ones (hidden exits once found, open timed exits): the mapper
learns nothing the text does not show. Exits use Mudlet's short names (`n`, `ne`, `u`,
`d`, ...); a map-change exit gives the room number on the other map. Until the client
sends `Core.Supports.Set`, every package goes out; after it, only the modules it lists
(`Core` always). `Core.Hello` is logged at debug level. A session that agreed to GMCP also
gets its text with backspaced letters taken out (`gmcp::apply_backspaces`): the direction
names' anti-script letter, which GMCP clients would otherwise show, since they apply no
backspaces.

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

`serve_with_rlogin` runs a second accept loop for `server.rlogin_bind` beside the telnet one;
the two share the session cap and the numbering. A connection from an address the
`RloginConfig` does not trust (loopback only when its list is empty) is closed before
anything is read. The handshake (`NUL client NUL server NUL term/speed NUL`, 10 seconds,
1 KiB at most) is read before the session is numbered, so a caller that never finishes it
leaves nothing in the gate or the engine; then `NUL` is sent and the session emits
`SessionEvent::Rlogin(hello)` just before `Connected`. An rlogin session sends no telnet
offers, doubles no 0xFF, strips no IAC and sends no heartbeat `NOP`: its bytes are plain.

## Dead connections

A player whose Wi-Fi drops, or whose laptop sleeps, leaves a connection that is still open
on the server and gone on their side. Nothing ever arrives on it, so without help its
character stays in the game, and the account `already logged on`, until
`game.inactive_secs` (an hour by default) kicks it.

Every accepted connection, telnet and rlogin, has TCP keepalive on (`keepalive.rs`): after
`net.keepalive_secs` (60) with no traffic either way, the server's system sends a probe,
then one every 15 seconds, and drops the connection after 4 unanswered, about two minutes
after the player's last sign of life. On Linux the same two minutes also bound output the
other end never acknowledges (`TCP_USER_TIMEOUT`). The session then ends as a hang-up does.
A live client's system answers the probes by itself, so an idle player is never
disconnected by them. `net.keepalive_secs = 0` turns the probes off.

A player who comes back sooner than that can take the account over from the dead connection
at login ([auth.md](auth.md#the-dialogue), `net.login_takeover`).

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
