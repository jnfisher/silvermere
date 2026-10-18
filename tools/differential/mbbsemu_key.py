#!/usr/bin/env python3
"""Give an MBBSEmu account a BBS key in its database (T-057), offline: the container stopped
and its `mbbsemu.db` copied out and back.

The original grants a channel sysop rights only when its user holds the key named by
SYSOPKEY (`wccsysop` in the stock WCCMMUD.MSG:46). MBBSEmu's
`-DBRESET sysop` account does not hold it, so every `SYSOP ...` of the m4 group became speech.

  mbbsemu_key.py DB USER KEY      add KEY to USER (idempotent); prints what it did

The tables are found by their columns rather than by name: an accounts table with a user name
column and an id, and a keys table with an account id column and a key column. Stdlib only.
"""
import sqlite3
import sys


def _columns(con, table):
    return [r[1] for r in con.execute(f'PRAGMA table_info("{table}")')]


def _find(con):
    """((accounts table, id column, name column), (keys table, account column, key column))."""
    tables = [r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")]
    accounts = keys = None
    for t in tables:
        cols = _columns(con, t)
        low = {c.lower(): c for c in cols}
        name = low.get("username") or low.get("user_name")
        ident = low.get("accountid") or low.get("account_id") or low.get("id")
        if name and ident and accounts is None:
            accounts = (t, ident, name)
        acc = low.get("accountid") or low.get("account_id")
        key = low.get("accountkey") or low.get("account_key") or low.get("keyname")
        if acc and key and keys is None:
            keys = (t, acc, key)
    if not accounts or not keys:
        raise SystemExit(f"no accounts/keys tables found among {tables}")
    return accounts, keys


def grant(db, user, key):
    con = sqlite3.connect(db)
    try:
        (at, aid, aname), (kt, kacc, kkey) = _find(con)
        row = con.execute(f'SELECT "{aid}" FROM "{at}" WHERE lower("{aname}") = lower(?)',
                          (user,)).fetchone()
        if row is None:
            raise SystemExit(f"no account {user!r} in {at}")
        acct = row[0]
        have = con.execute(f'SELECT 1 FROM "{kt}" WHERE "{kacc}" = ? AND upper("{kkey}") = upper(?)',
                           (acct, key)).fetchone()
        if have:
            return f"{user} already holds {key}"
        cols = _columns(con, kt)
        values = {kacc: acct, kkey: key.upper()}
        # date columns MBBSEmu fills on insert, filled the same way when NOT NULL
        for c in cols:
            if c not in values and c.lower() in ("createdate", "updatedate", "created", "updated"):
                values[c] = con.execute("SELECT datetime('now')").fetchone()[0]
        names = ", ".join(f'"{c}"' for c in values)
        marks = ", ".join("?" for _ in values)
        con.execute(f'INSERT INTO "{kt}" ({names}) VALUES ({marks})', list(values.values()))
        con.commit()
        return f"granted {key.upper()} to {user} ({kt})"
    finally:
        con.close()


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 3:
        raise SystemExit(__doc__)
    print(grant(*argv))
    return 0


if __name__ == "__main__":
    sys.exit(main())
