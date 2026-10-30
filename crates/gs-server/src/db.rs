//! Postgres: long-lived records (players, match results, leaderboard).
//! Schema lives in `/migrations` and is applied on startup.

use std::time::Duration;

use gs_core::MatchResult;
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Clone)]
pub struct Db {
    pool: Option<PgPool>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct LeaderRow {
    pub name: String,
    pub matches: i64,
    pub wins: i64,
    pub total_score: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MatchRow {
    pub id: i64,
    pub room: String,
    pub mode: String,
    pub summary: String,
    pub ended_at: String,
}

impl Db {
    pub async fn connect(url: Option<&str>) -> anyhow::Result<Db> {
        let Some(url) = url else {
            tracing::warn!("DATABASE_URL unset: match results will not be persisted");
            return Ok(Db { pool: None });
        };
        let mut attempt = 0;
        let pool = loop {
            match PgPoolOptions::new().max_connections(8).acquire_timeout(Duration::from_secs(3)).connect(url).await {
                Ok(p) => break p,
                Err(e) if attempt < 30 => {
                    attempt += 1;
                    tracing::warn!("postgres not ready ({e}); retrying");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e.into()),
            }
        };
        sqlx::migrate!("../../migrations").run(&pool).await?;
        tracing::info!("connected to postgres, migrations applied");
        Ok(Db { pool: Some(pool) })
    }

    pub fn enabled(&self) -> bool {
        self.pool.is_some()
    }

    /// Persist one finished round. `names` maps standings' player ids to display names.
    pub async fn record_match(
        &self,
        room: &str,
        mode: &str,
        result: &MatchResult,
        names: &[(u16, String, bool)],
    ) -> anyhow::Result<()> {
        let Some(pool) = &self.pool else { return Ok(()) };
        let mut tx = pool.begin().await?;
        let (match_id,): (i64,) =
            sqlx::query_as("INSERT INTO matches (room, mode, summary) VALUES ($1, $2, $3) RETURNING id")
                .bind(room)
                .bind(mode)
                .bind(&result.summary)
                .fetch_one(&mut *tx)
                .await?;
        for (rank, (pid, score)) in result.standings.iter().enumerate() {
            let Some((_, name, bot)) = names.iter().find(|(id, _, _)| id == pid) else { continue };
            sqlx::query(
                "INSERT INTO players (name, is_bot) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET last_seen = now()",
            )
            .bind(name)
            .bind(*bot)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO match_players (match_id, player_name, score, rank) VALUES ($1, $2, $3, $4)")
                .bind(match_id)
                .bind(name)
                .bind(*score)
                .bind(rank as i32 + 1)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn leaderboard(&self, limit: i64) -> anyhow::Result<Vec<LeaderRow>> {
        let Some(pool) = &self.pool else { return Ok(vec![]) };
        Ok(sqlx::query_as::<_, LeaderRow>(
            "SELECT mp.player_name AS name, COUNT(*) AS matches,
                    COUNT(*) FILTER (WHERE mp.rank = 1 AND mp.score > 0) AS wins,
                    COALESCE(SUM(mp.score), 0)::BIGINT AS total_score
             FROM match_players mp JOIN players p ON p.name = mp.player_name
             WHERE NOT p.is_bot
             GROUP BY mp.player_name
             ORDER BY total_score DESC, wins DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?)
    }

    pub async fn recent_matches(&self, limit: i64) -> anyhow::Result<Vec<MatchRow>> {
        let Some(pool) = &self.pool else { return Ok(vec![]) };
        Ok(sqlx::query_as::<_, MatchRow>(
            "SELECT id, room, mode, summary, to_char(ended_at, 'YYYY-MM-DD HH24:MI:SS') AS ended_at
             FROM matches ORDER BY id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?)
    }
}
