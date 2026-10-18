# Running silvermere with Docker

Telnet on `:2323`, admin API on `127.0.0.1:8080`, data in the `silvermere-data` volume.
Run everything from the repository root.

```sh
# 1. replace the placeholder SILVERMERE_ADMIN_TOKEN in docker/.env (the example value
#    is not a secret and must not be used as is)
cp docker/.env.example docker/.env

# 2. import the world first: the server exits on an empty database. The image holds no
#    game data: this downloads the stock 1.11p distribution (the MBBSEmu package, 6.7 MB)
#    once into the data volume (/data/cache), checks its sha256 and builds the world from
#    it. Offline, put the package zip in docker/legacy/ and add
#    `import /data/legacy/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` after `run --rm import`; for your
#    own files, put them in docker/legacy/ and add `import --from /data/legacy` (and `--live`
#    for a running realm's .DAT files)
docker compose -f docker/compose.yaml --profile import run --rm import

# 2b. then bring it up
docker compose -f docker/compose.yaml up -d --wait

# 3. logs
docker compose -f docker/compose.yaml logs -f silvermere

# 4. backup (written to /data/backups in the volume)
docker compose -f docker/compose.yaml exec silvermere /silvermere backup

# 5. switch to PostgreSQL: uncomment the `postgres` service, `pg-data`, `depends_on`
#    and the SILVERMERE_DATABASE_URL line in both `silvermere` and `import`
#    (import writes to the same database); the new database is empty, so run the import
#    (step 2) before `up`
docker compose -f docker/compose.yaml up -d --wait postgres
docker compose -f docker/compose.yaml --profile import run --rm import
docker compose -f docker/compose.yaml up -d --wait
```

A fresh `serve` with no imported world prints a message pointing at `import`.

## Settings used (from `silvermere serve --print-config`)

Nested keys use a double underscore: `SILVERMERE_SERVER__TELNET_BIND`,
`SILVERMERE_SERVER__ADMIN_BIND`, `SILVERMERE_LOGGING__FORMAT`. Flat names:
`SILVERMERE_DATABASE_URL`, `SILVERMERE_DATA_DIR`, and `SILVERMERE_ADMIN_TOKENS`
(a JSON array of `{name, token, scopes}`; compose builds it from `SILVERMERE_ADMIN_TOKEN`).
A non-loopback admin bind refuses to start without a token.

The image is distroless (no shell, no curl), runs as UID 65532, and health-checks itself with
`/silvermere healthcheck`. `editor` and `web` in the compose file are commented seams for
companion tools, not shipped images.
