#![allow(dead_code)]
//! A minimal reproduction of an implementation where an external crate's
//! trait constraints blocks an `Actor` when the internal method `handle` is called.
//! This strategy forwards the `Future`s created by `handle` so that they can
//! be `await`ed elsewhere in the program, unblocking the `handle` method for
//! an `Actor`, in this case a `Batcher`.
use futures::{
    future::BoxFuture,
    stream::{FuturesUnordered, StreamExt},
};
use std::sync::Arc;
use thiserror::Error;
use tokio::{self, sync::Mutex};

#[derive(Error, Debug)]
enum BatcherError {
    #[error("failed")]
    Error,
}

#[derive(Clone)]
enum BatcherMessage {
    GetNextBatch,
}

struct Batcher {
    future_pool: FuturesUnordered<BoxFuture<'static, Result<(), BatcherError>>>,
}
impl Batcher {
    fn handle_next_batch_request(
        batch: Arc<Mutex<Batcher>>,
    ) -> BoxFuture<'static, Result<(), BatcherError>> {
        Box::pin(Batcher::do_something(batch))
    }
    async fn do_something(_batch: Arc<Mutex<Batcher>>) -> Result<(), BatcherError> {
        Ok(())
    }
    fn new() -> Self {
        Self {
            future_pool: FuturesUnordered::new(),
        }
    }
}

// defined in ractor crate
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
struct BatcherActor;
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
                let batcher = state.lock().await;
                batcher.future_pool.push(fut);
                println!("sent");
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut batcher = Arc::new(Mutex::new(Batcher::new()));
    let batcher_actor = BatcherActor;
    let message = BatcherMessage::GetNextBatch;
    batcher_actor
        .handle(message.clone(), message.clone(), &mut batcher)
        .await
        .unwrap();
    let pool = tokio_rayon::rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    // We don't await the task here since we want to continuously poll
    // for `Future`s to pass to the future handler.
    let batcher_clone = batcher.clone();
    tokio::task::spawn(async move {
        loop {
            let mut guard = batcher_clone.lock().await;
            tokio::select! {
                fut = guard.future_pool.next() => {
                    if let Some(Ok(task)) = fut {
                        pool.install(|| async move {
                            println!("received");
                            task
                        })
                        .await;
                    }
                }
            }
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

    // Program must keep running so that the thread will continue to process incoming `Future`s.
    // Adding `thread::sleep` to reduce wasted cpu cycles.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
