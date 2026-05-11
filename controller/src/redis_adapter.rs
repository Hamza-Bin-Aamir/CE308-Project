use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use anyhow::Result;

pub type RedisPool = Pool;

pub fn create_pool_from_url(url: &str) -> Result<RedisPool> {
    let cfg = RedisConfig::from_url(url);
    Ok(cfg.create_pool(Some(Runtime::Tokio1))?)
}

pub async fn set_alert_key(pool: &RedisPool, key: &str, ttl_secs: usize) -> Result<()> {
    let mut conn = pool.get().await?;
    let _: () = deadpool_redis::redis::cmd("SETEX")
        .arg(key)
        .arg(ttl_secs)
        .arg(1u8)
        .query_async(&mut *conn)
        .await?;
    Ok(())
}
