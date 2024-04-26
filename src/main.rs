#![allow(dead_code)]
//! A minimal reproduction of an implementation where an external crate's
//! trait constraints blocks an `Actor` when the internal method `handle` is called.
//! This strategy forwards the `Future`s created by `handle` so that they can
//! be `await`ed elsewhere in the program, unblocking the `handle` method for
//! an `Actor`, in this case a `BatcherActor`.
use futures::{
    future::BoxFuture,
    stream::{FuturesUnordered, StreamExt},
    FutureExt,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::{self, sync::Mutex};

struct Batcher;
impl Batcher {
    async fn handle_next_batch_request(batch: Arc<Mutex<Batcher>>) -> Result<(), BatcherError> {
        Batcher::do_something(batch).await
    }
    async fn do_something(_batch: Arc<Mutex<Batcher>>) -> Result<(), BatcherError> {
        Ok(())
    }
    fn new() -> Self {
        Self
    }
}

#[derive(Error, Debug)]
enum BatcherError {
    #[error("failed")]
    Error,
}

#[derive(Clone)]
enum BatcherMessage {
    GetNextBatch,
}

#[derive(Clone)]
struct BatcherActor {
    future_pool: Arc<Mutex<FuturesUnordered<BoxFuture<'static, Result<(), BatcherError>>>>>,
}
impl BatcherActor {
    fn new() -> Self {
        Self {
            future_pool: Arc::new(Mutex::new(FuturesUnordered::new())),
        }
    }
    fn future_pool(
        &self,
    ) -> Arc<Mutex<FuturesUnordered<BoxFuture<'static, Result<(), BatcherError>>>>> {
        self.future_pool.clone()
    }
}
impl Actor for BatcherActor {
    type Msg = BatcherMessage;
    type State = Arc<Mutex<Batcher>>;
    async fn handle(
        &self,
        _myself: Self::Msg,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            BatcherMessage::GetNextBatch => {
                let fut = Batcher::handle_next_batch_request(Arc::clone(state));
                let guard = self.future_pool.lock().await;
                guard.push(fut.boxed());
            }
        }
        Ok(())
    }
}

// This is a minimal repro of types defined in the `ractor` crate.
// It does not reflect all requirements, only what is necessary
// for the `handle` method.
trait Actor: Sized + Send + Sync + 'static {
    type Msg;
    type State;
    async fn handle(
        &self,
        myself: Self::Msg,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr>;
}
#[derive(Error, Debug)]
enum ActorProcessingErr {
    #[error("failed")]
    Error,
}

#[tokio::main]
async fn main() {
    let mut batcher = Arc::new(Mutex::new(Batcher::new()));
    let batcher_actor = BatcherActor::new();
    let message = BatcherMessage::GetNextBatch;
    batcher_actor
        .handle(message.clone(), message.clone(), &mut batcher)
        .await
        .unwrap();
    {
        let guard = batcher_actor.future_pool.lock().await;
        assert!(!guard.is_empty());
    }

    let pool = tokio_rayon::rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus::get())
        .build()
        .unwrap();
    // We don't await the task here since we want to continuously poll
    // for `Future`s to pass to the future handler.
    let actor_clone = batcher_actor.clone(); // cloning here uses Arc::clone for the future_pool
                                             // so we can be sure we are still pointing to the same memory
    tokio::task::spawn(async move {
        loop {
            let futures = actor_clone.future_pool();
            let mut guard = futures.lock().await;
            pool.install(|| async move {
                if let Some(Err(err)) = guard.next().await {
                    println!("failed: {err:?}");
                }
            })
            .await;
        }
    });
    batcher_actor
        .handle(message.clone(), message.clone(), &mut batcher)
        .await
        .unwrap();
    batcher_actor
        .handle(message.clone(), message.clone(), &mut batcher)
        .await
        .unwrap();

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    loop {
        {
            let guard = batcher_actor.future_pool.lock().await;
            if guard.is_empty() {
                break;
            }
        }
        interval.tick().await;
    }
}
