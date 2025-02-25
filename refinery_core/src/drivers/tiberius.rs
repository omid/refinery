use crate::traits::r#async::{AsyncMigrate, AsyncQuery, AsyncTransaction};
use crate::Migration;

use async_trait::async_trait;
use chrono::{DateTime, Local};
use futures::{
    io::{AsyncRead, AsyncWrite},
    TryStreamExt,
};
use tiberius_driver::{error::Error, Client, QueryItem};

async fn query_applied_migrations<S: AsyncRead + AsyncWrite + Unpin + Send>(
    client: &mut Client<S>,
    query: &str,
) -> Result<Vec<Migration>, Error> {
    let mut rows = client.simple_query(query).await?;
    let mut applied = Vec::new();
    // Unfortunately too many unwraps as `Row::get` maps to Option<T> instead of T
    while let Some(item) = rows.try_next().await? {
        if let QueryItem::Row(row) = item {
            let version = row.get::<i32, usize>(0).unwrap();
            let applied_on: &str = row.get::<&str, usize>(2).unwrap();
            let applied_on = DateTime::parse_from_rfc3339(applied_on)
                .unwrap()
                .with_timezone(&Local);
            let checksum: String = row.get::<&str, usize>(3).unwrap().to_string();

            applied.push(Migration::applied(
                version,
                row.get::<&str, usize>(1).unwrap().to_string(),
                applied_on,
                checksum
                    .parse::<u64>()
                    .expect("checksum must be a valid u64"),
            ));
        }
    }

    Ok(applied)
}

#[async_trait]
impl<S> AsyncTransaction for Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    type Error = Error;

    async fn execute(&mut self, queries: &[&str]) -> Result<usize, Self::Error> {
        // Tiberius doesn't support transactions, see https://github.com/prisma/tiberius/issues/28
        self.simple_query("BEGIN TRAN T1;").await?;
        let mut count = 0;
        for query in queries {
            if let Err(err) = self.execute(*query, &[]).await {
                if let Err(err) = self.simple_query("ROLLBACK TRAN T1").await {
                    log::error!("could not ROLLBACK transaction, {}", err);
                }
                return Err(err);
            }
            count += 1;
        }
        self.simple_query("COMMIT TRAN T1").await?;
        Ok(count as usize)
    }
}

#[async_trait]
impl<S> AsyncQuery<Vec<Migration>> for Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        let applied = query_applied_migrations(self, query).await?;
        Ok(applied)
    }
}

impl<S> AsyncMigrate for Client<S> where S: AsyncRead + AsyncWrite + Unpin + Send {}
