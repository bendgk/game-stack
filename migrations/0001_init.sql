-- game-stack: long-lived records. Room state itself lives in Dragonfly (see store.rs).
-- Add new migrations as 0002_*.sql, 0003_*.sql… — never edit an applied one.

CREATE TABLE IF NOT EXISTS players (
    name        TEXT PRIMARY KEY,
    is_bot      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS matches (
    id          BIGSERIAL PRIMARY KEY,
    room        TEXT NOT NULL,
    mode        TEXT NOT NULL,
    summary     TEXT NOT NULL,
    ended_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS match_players (
    match_id    BIGINT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_name TEXT NOT NULL REFERENCES players(name),
    score       INTEGER NOT NULL,
    rank        INTEGER NOT NULL,
    PRIMARY KEY (match_id, player_name)
);

CREATE INDEX IF NOT EXISTS match_players_by_player ON match_players (player_name);
