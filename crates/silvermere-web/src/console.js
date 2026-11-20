// The sysop's console. Reads come from the admin API through /console/api/; every action is
// a command line run through /console/line, the same commands `silvermere sysop` takes, and
// is shown in the command drawer with its answer. Whatever a player wrote (names, reasons,
// gossip) is only ever set as text, never as HTML.
(function () {
  "use strict";
  var root = document.getElementById("console");
  if (!root) return;
  var base = root.dataset.base || "";
  var csrf = root.dataset.csrf;
  var $ = function (id) { return document.getElementById(id); };

  // ---- talking to the server ------------------------------------------------------------

  function signedOut() {
    var conn = $("conn");
    conn.textContent = "Your session has ended. ";
    var a = document.createElement("a");
    a.href = base + "/login";
    a.textContent = "Sign in again";
    conn.appendChild(a);
    conn.hidden = false;
  }

  // a background refresh does not keep the session from going idle
  function api(path, poll) {
    var headers = {};
    if (poll) headers["X-Console-Poll"] = "1";
    return fetch(base + "/console/api/" + path, { headers: headers, credentials: "same-origin" })
      .then(function (r) {
        if (r.status === 401) { signedOut(); throw new Error("signed out"); }
        return r.json();
      });
  }

  function post(url, body) {
    return fetch(base + url, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify(body)
    }).then(function (r) {
      return r.json().then(function (j) { return { status: r.status, body: j }; });
    });
  }

  // ---- the password, again --------------------------------------------------------------

  var dialog = $("password");
  function askPassword() {
    return new Promise(function (resolve) {
      $("password-error").hidden = true;
      $("password-input").value = "";
      var form = $("password-form");
      var done = function (ok) {
        form.onsubmit = null;
        $("password-cancel").onclick = null;
        dialog.close();
        resolve(ok);
      };
      form.onsubmit = function (e) {
        e.preventDefault();
        post("/console/password", { password: $("password-input").value }).then(function (r) {
          if (r.status === 200) { done(true); return; }
          var err = $("password-error");
          err.textContent = (r.body && r.body.message) || "Not accepted.";
          err.hidden = false;
        });
      };
      $("password-cancel").onclick = function () { done(false); };
      dialog.showModal();
      $("password-input").focus();
    });
  }

  // ---- the command line ------------------------------------------------------------------

  var out = $("out");
  function print(text, cls) {
    var line = document.createElement("div");
    if (cls) line.className = cls;
    line.textContent = text;
    out.appendChild(line);
    out.scrollTop = out.scrollHeight;
  }

  // a word as the command line reads it: bare when it can be, else in double quotes
  function q(s) {
    s = String(s);
    if (/^[A-Za-z0-9._:\/@-]+$/.test(s)) return s;
    return '"' + s.replace(/\\/g, "\\\\").replace(/"/g, '\\"') + '"';
  }

  function run(line) {
    print("sysop> " + line, "cmd");
    return post("/console/line", { line: line }).then(function (r) {
      if (r.status === 401 && r.body.error === "password") {
        return askPassword().then(function (ok) {
          if (!ok) { print("Not done: the password was not given.", "err"); return false; }
          return post("/console/line", { line: line }).then(show);
        });
      }
      if (r.status === 401) { signedOut(); return false; }
      return show(r);
    }).then(function (ok) { refreshSoon(); return ok; });
  }

  function show(r) {
    if (r.status === 200) { print(r.body.text.replace(/\n$/, ""), "ok"); return true; }
    print((r.body && r.body.message) || "Failed.", "err");
    return false;
  }

  var history = [];
  var at = 0;
  $("line").addEventListener("submit", function (e) {
    e.preventDefault();
    var cmd = $("cmd").value.trim();
    if (!cmd) return;
    history.push(cmd);
    at = history.length;
    $("cmd").value = "";
    run(cmd);
  });
  $("cmd").addEventListener("keydown", function (e) {
    if (e.key === "ArrowUp" && at > 0) { at -= 1; $("cmd").value = history[at]; e.preventDefault(); }
    if (e.key === "ArrowDown") {
      at = Math.min(history.length, at + 1);
      $("cmd").value = history[at] || "";
      e.preventDefault();
    }
  });
  // on a narrow screen the drawer starts closed: it would cover half of it
  if (window.matchMedia && window.matchMedia("(max-width: 60rem)").matches) {
    $("drawer").classList.add("closed");
    $("drawer-toggle").setAttribute("aria-expanded", "false");
  }
  $("drawer-toggle").addEventListener("click", function () {
    var open = $("drawer").classList.toggle("closed");
    $("drawer-toggle").setAttribute("aria-expanded", String(!open));
  });

  // ---- helpers for the panels ------------------------------------------------------------

  function el(tag, text, cls) {
    var e = document.createElement(tag);
    if (text !== undefined && text !== null) e.textContent = text;
    if (cls) e.className = cls;
    return e;
  }

  function when(iso) {
    if (!iso) return "until lifted";
    var d = new Date(iso);
    return "until " + d.toLocaleString();
  }

  function idle(secs) {
    var m = Math.floor(secs / 60);
    var s = secs % 60;
    return m + ":" + (s < 10 ? "0" : "") + s;
  }

  function button(text, onclick, cls) {
    var b = el("button", text, cls);
    b.type = "button";
    b.addEventListener("click", onclick);
    return b;
  }

  // ---- the state of the server -----------------------------------------------------------

  var state = { players: [], sanctions: [], ipBans: [], realmClosed: false, plan: null };
  var selected = null; // { name, ip }

  function sanctionsOf(name) {
    var n = name.toLowerCase();
    return state.sanctions.filter(function (s) {
      return (s.character || "").toLowerCase() === n || (s.username || "").toLowerCase() === n;
    });
  }

  function renderPlayers() {
    var body = $("players").tBodies[0];
    body.textContent = "";
    state.players.forEach(function (p) {
      var tr = document.createElement("tr");
      var name = (p.first_name + " " + (p.last_name || "")).trim();
      var flags = [];
      if (p.jailed) flags.push("jailed");
      if (p.muted) flags.push("muted");
      if (p.hidden) flags.push("hidden");
      tr.appendChild(el("td", name));
      var room = el("td", p.room_name || "", "room");
      if (p.room) room.appendChild(el("span", " " + p.room, "where"));
      tr.appendChild(room);
      [p.level, p.class, p.race, idle(p.idle_secs), p.ip || ""].forEach(function (v) {
        tr.appendChild(el("td", v));
      });
      tr.appendChild(el("td", flags.join(", "), "flags"));
      tr.tabIndex = 0;
      tr.addEventListener("click", function () { select(p.first_name, p.ip, p); });
      tr.addEventListener("keydown", function (e) { if (e.key === "Enter") select(p.first_name, p.ip, p); });
      if (selected && selected.name.toLowerCase() === p.first_name.toLowerCase()) tr.className = "here";
      body.appendChild(tr);
    });
    $("count").textContent = "(" + state.players.length + ")";
    $("nobody").hidden = state.players.length > 0;
    $("st-players").textContent = String(state.players.length);
  }

  function select(name, ip, row) {
    selected = { name: name, ip: ip || null };
    $("detail").hidden = false;
    $("d-name").textContent = name;
    var meta = row
      ? "Level " + row.level + " " + row.race + " " + row.class +
        (row.gang ? ", " + row.gang : "") +
        (row.room ? " · in " + row.room_name + " (" + row.room + ")" : "") +
        " · idle " + idle(row.idle_secs) +
        (row.ip ? " · from " + row.ip : "") + " · account " + row.account
      : "Not in the realm.";
    $("d-meta").textContent = meta;
    var on = sanctionsOf(name).map(function (s) { return s.kind + " " + when(s.until); });
    $("d-flags").textContent = on.length ? "In force: " + on.join("; ") : "Nothing in force.";
    $("d-ban-ip").hidden = !selected.ip;
    renderPlayers();
  }

  function renderModeration(m) {
    state.realmClosed = m.realm.closed;
    state.sanctions = m.sanctions;
    state.ipBans = m.ip_bans;
    $("st-realm").textContent = m.realm.closed
      ? "closed" + (m.realm.reason ? " (" + m.realm.reason + ")" : "")
      : "open";
    $("st-realm").className = m.realm.closed ? "warn" : "ok";
    $("b-realm").textContent = m.realm.closed ? "Open realm" : "Close realm…";
    var list = $("sanctions");
    list.textContent = "";
    m.sanctions.forEach(function (s) {
      var who = s.character || s.username;
      var li = el("li");
      li.appendChild(el("b", s.kind + " "));
      var a = el("a", who);
      a.href = "#";
      a.addEventListener("click", function (e) { e.preventDefault(); select(who, null, findRow(who)); });
      li.appendChild(a);
      li.appendChild(el("span", " " + when(s.until) + (s.reason ? ": " + s.reason : "") + " (by " + s.by + ") "));
      var undo = { ban: "unban", jail: "release", mute: "unmute" }[s.kind];
      li.appendChild(button("Lift", function () { run(undo + " " + q(who)); }, "link"));
      list.appendChild(li);
    });
    m.ip_bans.forEach(function (b) {
      var li = el("li");
      li.appendChild(el("b", "ip "));
      li.appendChild(el("span", b.network + " " + when(b.until) + (b.reason ? ": " + b.reason : "") + " (by " + b.by + ") "));
      li.appendChild(button("Lift", function () { run("unban-ip " + q(b.network)); }, "link"));
      list.appendChild(li);
    });
    $("no-sanctions").hidden = m.sanctions.length + m.ip_bans.length > 0;
    if (selected) select(selected.name, selected.ip, findRow(selected.name));
  }

  function findRow(name) {
    var n = name.toLowerCase();
    for (var i = 0; i < state.players.length; i++) {
      if (state.players[i].first_name.toLowerCase() === n) return state.players[i];
    }
    return null;
  }

  function renderBackups(b) {
    var last = b.last;
    $("st-backup").textContent = last
      ? (last.ok ? "ok, " : "FAILED, ") + new Date(last.at).toLocaleString()
      : "none yet";
    $("st-backup").className = last && !last.ok ? "warn" : "";
    $("backup-sched").textContent = b.enabled
      ? "Daily at " + b.at + " (" + b.tz + "), keeping " + b.keep + (b.next ? "; next " + new Date(b.next).toLocaleString() : "")
      : "Scheduled backups are off (backup.enabled).";
    var list = $("backups");
    list.textContent = "";
    (b.backups || []).slice(0, 10).forEach(function (f) {
      list.appendChild(el("li", f.name + " · " + Math.round(f.bytes / 1024) + " KiB"));
    });
  }

  function renderPlan(p) {
    state.plan = p.planned;
    var plan = p.planned;
    $("st-plan").textContent = plan
      ? (plan.restart ? "restart" : "shutdown") + " in " + idle(plan.in_secs)
      : "nothing";
    $("st-plan").className = plan ? "warn" : "";
    $("b-cancel").hidden = !plan;
  }

  function refresh() {
    api("v1/who", true).then(function (w) { state.players = w.players; renderPlayers(); }).catch(function () {});
    api("v1/moderation", true).then(renderModeration).catch(function () {});
    api("v1/backup", true).then(renderBackups).catch(function () {});
    api("v1/shutdown", true).then(renderPlan).catch(function () {});
    api("healthz", true).then(function (h) {
      $("st-health").textContent = h.status === "ok" ? "running" : "unhealthy (" + h.engine + ", " + h.persist + ")";
      $("st-health").className = h.status === "ok" ? "ok" : "warn";
    }).catch(function () {});
  }

  var soon = null;
  function refreshSoon() {
    if (soon) return;
    soon = setTimeout(function () { soon = null; refresh(); }, 400);
  }

  // ---- buttons ---------------------------------------------------------------------------

  function forPart() {
    var f = $("d-for").value;
    return f ? " --for " + f : "";
  }
  function reasonPart() {
    var r = $("d-reason").value.trim();
    return r ? " " + q(r) : "";
  }

  document.querySelectorAll("#detail [data-act]").forEach(function (b) {
    b.addEventListener("click", function () {
      if (!selected) return;
      var who = q(selected.name);
      var act = b.dataset.act;
      if (act === "close") { selected = null; $("detail").hidden = true; renderPlayers(); return; }
      var line = {
        kick: "kick " + who + reasonPart(),
        jail: "jail " + who + forPart() + reasonPart(),
        release: "release " + who,
        mute: "mute " + who + forPart() + reasonPart(),
        unmute: "unmute " + who,
        ban: "ban " + who + forPart() + reasonPart(),
        unban: "unban " + who,
        "ban-ip": "ban-ip " + q(selected.ip || "") + forPart() + reasonPart()
      }[act];
      if ((act === "ban" || act === "ban-ip" || act === "kick") &&
          !window.confirm("Run: " + line + " ?")) return;
      run(line);
    });
  });

  $("d-tell").addEventListener("submit", function (e) {
    e.preventDefault();
    if (!selected) return;
    run("tell " + q(selected.name) + " " + q($("d-tell-text").value)).then(function (ok) {
      if (ok) $("d-tell-text").value = "";
    });
  });

  $("find").addEventListener("submit", function (e) {
    e.preventDefault();
    var name = $("find-name").value.trim();
    if (name) select(name, null, findRow(name));
  });

  $("ban-ip").addEventListener("submit", function (e) {
    e.preventDefault();
    var net = $("ban-ip-net").value.trim();
    if (net && window.confirm("Run: ban-ip " + net + " ?")) {
      run("ban-ip " + q(net)).then(function (ok) { if (ok) $("ban-ip-net").value = ""; });
    }
  });

  $("b-realm").addEventListener("click", function () {
    if (state.realmClosed) { run("realm open"); return; }
    var reason = window.prompt("Close the realm: every player but the sysops is saved and disconnected. Reason (told to them):", "");
    if (reason === null) return;
    run("realm close" + (reason.trim() ? " " + q(reason.trim()) : ""));
  });

  $("b-backup").addEventListener("click", function () { run("backup"); });

  function plan(what) {
    var when = window.prompt("The server will " + what + " after warning the players. In how long? (90s, 5m, 1h)", "5m");
    if (when === null || !when.trim()) return;
    var msg = window.prompt("A message for the players (optional):", "");
    if (msg === null) return;
    run((what === "restart" ? "restart" : "shutdown") + " --in " + q(when.trim()) + (msg.trim() ? " " + q(msg.trim()) : ""));
  }
  $("b-restart").addEventListener("click", function () { plan("restart"); });
  $("b-shutdown").addEventListener("click", function () { plan("shut down"); });
  $("b-cancel").addEventListener("click", function () {
    run((state.plan && state.plan.restart ? "restart" : "shutdown") + " --cancel");
  });

  // ---- the activity feed -----------------------------------------------------------------

  var feed = $("feed");
  var wanted = {};
  function readFilters() {
    document.querySelectorAll("#filters input").forEach(function (c) { wanted[c.value] = c.checked; });
  }
  readFilters();
  document.querySelectorAll("#filters input").forEach(function (c) {
    c.addEventListener("change", function () { readFilters(); applyFilters(); });
  });

  function family(kind) {
    var head = kind.split(".")[0];
    if (head === "logoff") return "login";
    if (head === "sysop") return "admin";
    if (head === "closed") return "connected";
    return head;
  }

  function applyFilters() {
    Array.prototype.forEach.call(feed.children, function (li) {
      li.hidden = !wanted[li.dataset.family];
    });
  }

  function describe(e) {
    var p = e.payload || {};
    switch (e.kind) {
      case "login": return p.name + " entered the realm" + (p.ip ? " from " + p.ip : "");
      case "logoff": return p.name + " left the realm (" + p.how + ")";
      case "death": return p.name + " died" + (e.room ? " in " + e.room.map + ":" + e.room.room : "");
      case "level": return "session " + e.session + " reached level " + p.level;
      case "connected": return "connection " + e.session + " opened";
      case "closed": return "connection " + e.session + " closed";
    }
    if (e.kind.indexOf("admin.") === 0) {
      return (p.token || "?") + ": " + p.method + " " + e.kind.slice(6) + " (" + p.status + ")" +
        (p.reason ? " — " + p.reason : "") + (p.text ? " — " + p.text : "");
    }
    if (e.kind.indexOf("auth.") === 0) {
      return e.kind.slice(5).replace(/_/g, " ") + ": " + (p.username || p.account || "") + (p.reason ? " (" + p.reason + ")" : "");
    }
    if (e.kind.indexOf("sysop.") === 0) return (p.user || "?") + ": " + e.kind.slice(6) + " " + (p.detail || "");
    return e.kind;
  }

  function addEvent(e) {
    var li = el("li");
    li.dataset.family = family(e.kind);
    li.appendChild(el("time", new Date(e.at).toLocaleTimeString()));
    li.appendChild(el("span", " " + describe(e)));
    li.hidden = !wanted[li.dataset.family];
    feed.insertBefore(li, feed.firstChild);
    while (feed.children.length > 300) feed.removeChild(feed.lastChild);
  }

  function listen() {
    // unnamed frames: one handler hears every kind
    var es = new EventSource(base + "/console/api/v1/events?plain=1");
    es.onmessage = function (m) {
      try {
        var e = JSON.parse(m.data);
        addEvent(e);
        if (e.kind !== "level" && e.kind.indexOf("auth.") !== 0) refreshSoon();
      } catch (err) { /* a frame that is not an event */ }
    };
    es.onerror = function () { lost(); };
    es.onopen = function () { found(); };
  }

  // ---- losing the server (a restart) -----------------------------------------------------

  var waiting = null;
  function lost() {
    if (waiting) return;
    $("conn").textContent = "The server is not answering (restarting?). Reconnecting…";
    $("conn").hidden = false;
    waiting = setInterval(function () {
      api("healthz", true).then(function (h) {
        if (h.status === "ok") found();
      }).catch(function () {});
    }, 2000);
  }
  function found() {
    if (!waiting) return;
    clearInterval(waiting);
    waiting = null;
    $("conn").hidden = true;
    print("The server is back.", "ok");
    refresh();
  }

  // who stands where changes without an event: the player list is read every 3 seconds
  // while the page is in view, the rest every 15
  function refreshPlayers() {
    if (document.hidden) return;
    api("v1/who", true).then(function (w) { state.players = w.players; renderPlayers(); }).catch(function () {});
  }
  refresh();
  listen();
  setInterval(refresh, 15000);
  setInterval(refreshPlayers, 3000);
  print("Signed in. Type help for the commands.", "hint");
})();
