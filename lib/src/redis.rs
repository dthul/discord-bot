use futures_util::FutureExt;
use rand::Rng;
use redis::{AsyncCommands, Commands};
use std::{future::Future, pin::Pin};

pub fn closure_type_helper<
    T: redis::FromRedisValue + Send + 'static,
    F: for<'c> FnMut(
        &'c mut redis::aio::MultiplexedConnection,
        redis::Pipeline,
    ) -> Pin<Box<dyn Future<Output = redis::RedisResult<Option<T>>> + Send + 'c>>,
>(
    func: F,
) -> F {
    func
}

// A direct translation of redis::transaction for the async case
pub async fn async_redis_transaction<
    K: redis::ToRedisArgs,
    T: redis::FromRedisValue + Send + 'static,
    F: for<'c> FnMut(
        &'c mut redis::aio::MultiplexedConnection,
        redis::Pipeline,
    ) -> Pin<Box<dyn Future<Output = redis::RedisResult<Option<T>>> + Send + 'c>>,
>(
    con: &mut redis::aio::MultiplexedConnection,
    keys: &[K],
    mut func: F,
) -> redis::RedisResult<T> {
    loop {
        let () = redis::cmd("WATCH").arg(keys).query_async(con).await?;
        let mut p = redis::pipe();
        p.atomic();
        let response: Option<T> = func(con, p).await?;
        match response {
            None => continue,
            Some(response) => {
                // make sure no watch is left in the connection, even if
                // someone forgot to use the pipeline.
                let () = redis::cmd("UNWATCH").query_async(con).await?;
                return Ok(response);
            }
        }
    }
}

pub struct Lock<'a> {
    redis_connection: &'a mut redis::Connection,
    lockname: &'a str,
    identifier: u64,
}

// https://redislabs.com/ebook/part-2-core-concepts/chapter-6-application-components-in-redis/6-2-distributed-locking/6-2-5-locks-with-timeouts/
impl<'a> Lock<'a> {
    pub fn acquire_with_timeout(
        redis_connection: &'a mut redis::Connection,
        lockname: &'a str,
        acquire_timeout: std::time::Duration,
        lock_timeout: std::time::Duration,
    ) -> Result<Option<Self>, crate::meetup::Error> {
        let identifier: u64 = rand::thread_rng().gen();
        let start = std::time::Instant::now();
        while start.elapsed() < acquire_timeout {
            let was_set: u8 = redis_connection.set_nx(lockname, identifier)?;
            if was_set == 1 {
                redis_connection.expire::<_, ()>(lockname, lock_timeout.as_secs() as i64)?;
                return Ok(Some(Lock {
                    redis_connection,
                    lockname,
                    identifier,
                }));
            } else {
                // We failed to get the lock.
                // Make sure that a timeout is set
                let current_timeout: isize = redis_connection.ttl(lockname)?;
                if current_timeout < 0 {
                    // No timeout set: set it now
                    redis_connection.expire::<_, ()>(lockname, lock_timeout.as_secs() as i64)?;
                }
            }
            // Sleep for a moment and re-try
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(None)
    }

    fn release(&mut self) -> Result<(), crate::meetup::Error> {
        let lockname = self.lockname;
        let identifier = self.identifier;
        let _: () = redis::transaction(self.redis_connection, &[self.lockname], |con, pipe| {
            let current_identifier: u64 = con.get(lockname)?;
            if current_identifier == identifier {
                pipe.del(lockname);
            }
            pipe.query(con)
        })?;
        Ok(())
    }

    pub fn con(&mut self) -> &mut redis::Connection {
        self.redis_connection
    }
}

impl<'a> Drop for Lock<'a> {
    fn drop(&mut self) {
        if let Err(err) = self.release() {
            eprintln!("Error when trying to release Redis lock:\n{:#?}", err);
        }
    }
}

pub struct AsyncLock<'a> {
    redis_connection: &'a mut redis::aio::MultiplexedConnection,
    lockname: &'a str,
    identifier: u64,
}

impl<'a> AsyncLock<'a> {
    pub async fn acquire_with_timeout(
        redis_connection: &'a mut redis::aio::MultiplexedConnection,
        lockname: &'a str,
        acquire_timeout: std::time::Duration,
        lock_timeout: std::time::Duration,
    ) -> Result<Option<AsyncLock<'a>>, crate::meetup::Error> {
        let identifier: u64 = rand::thread_rng().gen();
        let start = std::time::Instant::now();
        while start.elapsed() < acquire_timeout {
            let was_set: u8 = redis_connection.set_nx(lockname, identifier).await?;
            if was_set == 1 {
                redis_connection
                    .expire::<_, ()>(lockname, lock_timeout.as_secs() as i64)
                    .await?;
                return Ok(Some(AsyncLock {
                    redis_connection,
                    lockname,
                    identifier,
                }));
            } else {
                // We failed to get the lock.
                // Make sure that a timeout is set
                let current_timeout: isize = redis_connection.ttl(lockname).await?;
                if current_timeout < 0 {
                    // No timeout set: set it now
                    redis_connection
                        .expire::<_, ()>(lockname, lock_timeout.as_secs() as i64)
                        .await?;
                }
            }
            // Sleep for a moment and re-try
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }
        Ok(None)
    }

    // There is no AsyncDrop yet, so release is manual
    pub async fn release(self) -> Result<(), crate::meetup::Error> {
        let lockname = self.lockname;
        let identifier = self.identifier;
        let _: () = async_redis_transaction(self.redis_connection, &[lockname], |con, mut pipe| {
            let lockname = lockname.to_string();
            async move {
                let current_identifier: u64 = con.get(&lockname).await?;
                if current_identifier == identifier {
                    pipe.del(&lockname);
                }
                pipe.query_async(con).await
            }
            .boxed()
        })
        .await?;
        Ok(())
    }

    pub fn con(&mut self) -> &mut redis::aio::MultiplexedConnection {
        self.redis_connection
    }
}
