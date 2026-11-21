-- ============================================================================
-- Moderation: bans, jail and mutes, IP bans, a closed realm
-- ============================================================================

-- What a sysop has put on an account: a ban (no login), a jail (held in the jail room) or a
-- mute (no gossip or auction). One row per account and kind; lifting it deletes the row. A
-- row whose until has passed is over and is ignored.
CREATE TABLE account_sanctions (
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,  -- the account
    kind       TEXT NOT NULL,                                               -- ban, jail or mute
    until      TEXT,                                                        -- RFC 3339 UTC when it ends; NULL until lifted
    reason     TEXT NOT NULL DEFAULT '',                                    -- told to the player
    by_whom    TEXT NOT NULL,                                               -- the sysop or admin token that set it
    created    TEXT NOT NULL,                                               -- RFC 3339 UTC
    PRIMARY KEY (account_id, kind)
);

-- Telnet connections from these addresses are closed before the login screen. The rlogin
-- door is never checked: its callers all come from the BBS.
CREATE TABLE ip_bans (
    id       BIGSERIAL PRIMARY KEY,             -- ban id
    network  TEXT NOT NULL,                     -- an address or CIDR block, as 203.0.113.0/24 or 2001:db8::/32
    until    TEXT,                              -- RFC 3339 UTC when it ends; NULL until lifted
    reason   TEXT NOT NULL DEFAULT '',          -- for the sysop; the caller is not told
    by_whom  TEXT NOT NULL,                     -- the sysop or admin token that set it
    created  TEXT NOT NULL                      -- RFC 3339 UTC
);
CREATE UNIQUE INDEX ip_bans_network ON ip_bans (network);

-- One row while the realm is closed: only sysops may log in. No row: open.
CREATE TABLE realm_closed (
    id       INTEGER PRIMARY KEY,      -- always 1
    reason   TEXT NOT NULL DEFAULT '', -- told to whoever is refused
    by_whom  TEXT NOT NULL,            -- the sysop or admin token that closed it
    created  TEXT NOT NULL             -- RFC 3339 UTC
);
